# Android Standalone vs iOS Runtime Gap Report

## Summary

| Area | Android | iOS | Severity | Note |
| --- | --- | --- | --- | --- |
| Startup / lazy init / restore UX | Cached torrent summaries + deferred engine start | No equivalent cache; list depends on live controller state | High | Missing optimization |
| UI/runtime integration | Repository + subscription replay/pause/resume | Direct `@MainActor` controller + redundant polling queries | High | Functionally correct but naive |
| Promise / async bridge | Callback-based await bridge with cancellation | Polling-based `awaitPromise()` loop | Medium | Functionally correct but naive |
| File I/O core | Pooled native + SAF handles, positioned I/O, validation | Per-op `FileHandle` open/seek/close | High | Missing optimization |
| Verified write batching | Parallel background writes + batched result flush | Batch transport exists, but writes are serialized and synced per write | High | Partial port |
| Session persistence backend | SQLite KV store used by engine + cache | `UserDefaults` string store with `synchronize()` | High | Partial port / naive |
| `verifyChunks` / recheck | Streaming verifier over pooled handles | Re-opens files and materializes chunk buffers | Medium | Functionally correct but naive |
| Socket close/data ordering | Flush pending TCP data before close/error callback | Close callback can race ahead of queued data flush | High | Partial port / correctness risk |
| Download root management | Live root sync + invalid root surfaces as error | Controller rebuild on root change; unknown roots silently map to sandbox path | High | Partial port |
| Background lifecycle | Selective suspend/shutdown/start based on cache + service state | Unconditional shutdown on app background | Low | Justified platform divergence, but poorly compensated |

## Detailed Findings

### 1. Lazy startup and cached restore UX are missing on iOS

- Classification: missing optimization
- Severity: High
- Android implementation:
  - The app loads cached torrent summaries without starting the engine via `TorrentSummaryCache` and overlays cache/live state in the list VM (`android/app/src/main/java/com/jstorrent/app/JSTorrentApplication.kt:71`, `android/app/src/main/java/com/jstorrent/app/cache/TorrentSummaryCache.kt:17`, `android/app/src/main/java/com/jstorrent/app/viewmodel/TorrentListViewModel.kt:64`, `android/app/src/main/java/com/jstorrent/app/viewmodel/TorrentListViewModel.kt:145`).
  - Activity startup explicitly avoids starting the engine and relies on on-demand triggers (`android/app/src/main/java/com/jstorrent/app/NativeStandaloneActivity.kt:128`, `android/app/src/main/java/com/jstorrent/app/NativeStandaloneActivity.kt:219`).
  - Background lifecycle can decide whether to restart the engine from cached work state (`android/app/src/main/java/com/jstorrent/app/service/ServiceLifecycleManager.kt:169`, `android/app/src/main/java/com/jstorrent/app/service/ServiceLifecycleManager.kt:202`).
- iOS implementation:
  - `AppModel` creates an `EngineController`, but the list screen renders only `controller.torrents`; there is no persisted summary/cache path (`ios/JSTorrent/App/AppModel.swift:16`, `ios/JSTorrent/App/TorrentListScreen.swift:67`, `ios/JSTorrent/App/TorrentListScreen.swift:91`).
  - The engine is started only when the user refreshes, adds/imports a torrent, or opens detail, and the app fully shuts the engine down on background (`ios/JSTorrent/App/TorrentListScreen.swift:162`, `ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/EngineController.swift:166`, `ios/JSTorrent/App/ContentView.swift:43`).
- Gap:
  - Android ports persisted session state into a fast cached UI path; iOS has only the live runtime path.
- Impact:
  - Cold launch and return-from-background can show an empty list until the engine is started again.
  - Repeated engine bootstrap becomes part of normal navigation instead of an exceptional path.
- Recommended fix:
  - Port a lightweight persisted torrent-summary cache on iOS and overlay it with live runtime state.
  - Keep the list UI decoupled from engine liveness the same way Android does.

### 2. iOS kept the subscription API but not Android’s repository/replay/pause architecture

- Classification: functionally correct but naive port
- Severity: High
- Android implementation:
  - `EngineServiceRepository` monitors controller replacement, replays subscriptions after restart, and pauses/resumes pushes when nobody is listening (`android/app/src/main/java/com/jstorrent/app/viewmodel/EngineServiceRepository.kt:89`, `android/app/src/main/java/com/jstorrent/app/viewmodel/EngineServiceRepository.kt:137`).
  - `SubscriptionTracker` is the shared source of truth for ref-counted subscriptions (`android/app/src/main/java/com/jstorrent/app/viewmodel/EngineServiceRepository.kt:91`).
  - Runtime state is pushed into `StateFlow` via the native event listener instead of repeatedly queried (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/EngineController.kt:167`, `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/EngineController.kt:1467`).
- iOS implementation:
  - `EngineController` is `@MainActor`, directly owns all runtime/UI state, and drives subscriptions itself (`ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/EngineController.swift:105`).
  - It subscribes to torrent updates, but still falls back to explicit query polling after commands and every 500ms from the tick loop (`ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/EngineController.swift:191`, `ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/EngineController.swift:463`, `ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/EngineController.swift:565`, `ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/EngineController.swift:613`).
  - Detail screens subscribe/unsubscribe directly from SwiftUI tasks with no shared replay/ref-count layer (`ios/JSTorrent/App/TorrentDetailScreen.swift:107`).
- Gap:
  - Android treats subscriptions as durable runtime infrastructure; iOS treats them as view-owned helpers and compensates with polling.
- Impact:
  - Extra FFI crossings, repeated JSON query/decode work, and more main-thread-visible latency.
  - Runtime restarts/root changes lose Android’s automatic subscription replay behavior.
- Recommended fix:
  - Port Android’s repository/subscription-tracker pattern to iOS.
  - Make pushed subscription payloads authoritative and remove `scheduleTorrentRefreshes()` plus tick-loop query polling.

### 3. Promise bridging on iOS is a polling loop, not Android’s callback-based bridge

- Classification: functionally correct but naive port
- Severity: Medium
- Android implementation:
  - `QuickJsEngine.callGlobalFunctionAwaitPromise()` installs temporary resolve/reject globals, resumes a coroutine when the JS promise settles, supports cancellation, and has a binary variant for streaming APIs (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/QuickJsEngine.kt:478`, `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/QuickJsEngine.kt:573`).
- iOS implementation:
  - `JSEngine.awaitPromise()` writes state into `globalThis[token]`, polls it by repeatedly evaluating JS, and spins the current run loop until timeout (`ios/JSTorrentKit/Sources/JSTorrentKit/Engine/JSEngine.swift:172`).
  - Runtime shutdown depends on that polling path (`ios/JSTorrentKit/Sources/JSTorrentKit/Runtime/JSTorrentRuntime.swift:260`).
- Gap:
  - Android has a true async bridge; iOS emulates one with repeated JS evaluation.
- Impact:
  - More JS re-entry and bridge overhead on every awaited command.
  - No native cancellation or structured cleanup comparable to Android’s pending-callback tracking.
- Recommended fix:
  - Port Android’s callback-based promise await bridge to iOS, including a binary-result path.
  - Remove the polling/run-loop loop from `JSEngine.awaitPromise()`.

### 4. Core file I/O pooling/caching was not ported to iOS

- Classification: missing optimization
- Severity: High
- Android implementation:
  - `FileManagerImpl` pools native file handles and SAF handles, uses positioned `FileChannel`/`pread`/`pwrite`, validates stale handles, and evicts idle entries (`android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:24`, `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:84`, `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:255`, `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:864`, `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:1052`).
  - Async native reads/writes are batched across the FFI boundary and dispatched on background workers (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/FileBindings.kt:776`, `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/FileBindings.kt:899`, `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/FileBindings.kt:1038`).
- iOS implementation:
  - Every `readFile()` opens a new `FileHandle`, seeks, reads, then closes it (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:683`).
  - Every `writeFile()` opens a new `FileHandle`, seeks, writes, synchronizes, then closes it (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:698`).
  - Root management is just a `[String: URL]` map; there is no native handle pool underneath (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:166`, `ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:191`).
- Gap:
  - The Android file-manager architecture did not make the jump to iOS; only the binding surface did.
- Impact:
  - High syscall/open-close churn during download, streaming, and recheck.
  - Lower throughput and more battery/CPU overhead than the Android standalone path.
- Recommended fix:
  - Introduce an iOS file-manager layer with pooled descriptors/handles, positioned I/O, validation/eviction, and root-aware caching.
  - Keep `FileBindings` as a thin bridge, as Android does.

### 5. Verified-write batching exists on iOS, but the backend is still serialized and over-synced

- Classification: partial port
- Severity: High
- Android implementation:
  - The TS disk queue batches writes into one native call (`packages/engine/src/adapters/native/native-batching-disk-queue.ts:1`).
  - Android native then launches each verified write on `Dispatchers.IO`, allowing batch members to hash/write concurrently (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/FileBindings.kt:926`).
- iOS implementation:
  - iOS parses the same batch format, but `queueVerifiedWrite()` dispatches onto a single serial `writeQueue` (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:179`, `ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:526`).
  - Each write still goes through `writeFile()`, which calls `FileHandle.synchronize()` per write (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:708`).
- Gap:
  - The FFI batching optimization was ported, but the native execution model stayed single-file/synchronous.
- Impact:
  - Batch mode reduces crossings but does not deliver Android-like throughput.
  - Per-piece sync amplifies latency and storage wear/energy cost.
- Recommended fix:
  - Use a bounded concurrent write worker pool on iOS, backed by pooled file handles.
  - Remove per-write `synchronize()`; flush on checkpoint/shutdown or on a coarse timer instead.

### 6. iOS still uses `UserDefaults` for session storage while Android moved to SQLite

- Classification: partial port / naive port
- Severity: High
- Android implementation:
  - Storage bindings are backed by `SqliteKVStore` specifically because session values can be large (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/StorageBindings.kt:9`).
  - The Android app also uses that SQLite store for config and cached UI state (`android/app/src/main/java/com/jstorrent/app/JSTorrentApplication.kt:110`, `android/app/src/main/java/com/jstorrent/app/cache/TorrentSummaryCache.kt:22`).
- iOS implementation:
  - Storage bindings are simple `UserDefaults` get/set/delete/keys wrappers and call `synchronize()` on set/delete (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/StorageBindings.swift:3`).
  - Shared session persistence stores torrent list/state, peers, `.torrent` bytes, and saved infodicts through this abstraction (`packages/engine/src/core/session-persistence.ts:122`, `packages/engine/src/core/session-persistence.ts:148`, `packages/engine/src/core/session-persistence.ts:204`, `packages/engine/src/core/session-persistence.ts:219`).
  - `NativeSessionStore` is written as if the native backend were SQLite-like (`packages/engine/src/adapters/native/native-session-store.ts:1`), but iOS does not provide that backend.
- Gap:
  - Android changed the persistence substrate; iOS kept the older small-key/value style backend.
- Impact:
  - Large JSON/base64 payloads are copied through `UserDefaults`.
  - Prefix scans (`storage_keys`) enumerate the whole defaults domain.
  - The same backend choice blocks Android-style cached-summary startup on iOS.
- Recommended fix:
  - Replace iOS `StorageBindings` with a SQLite KV store and drop eager `synchronize()`.
  - Build the iOS summary cache on top of the same store.

### 7. `verifyChunks` / recheck uses Android’s API shape but not its streaming implementation

- Classification: functionally correct but naive port
- Severity: Medium
- Android implementation:
  - `verifyChunks()` walks the concatenated file layout once, hashes in streaming chunks, and uses pooled read handles; the read-only handle path explicitly avoids creating files during recheck (`android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:463`, `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt:938`).
  - Torrent recheck prefers this batched native path before falling back (`packages/engine/src/core/torrent.ts:4277`, `packages/engine/src/core/torrent.ts:4316`).
- iOS implementation:
  - `verifyChunks()` repeatedly calls `readConcatenatedChunk()`, which re-opens files through `readFile()` and assembles each chunk into a new `Data` buffer before hashing (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:747`, `ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:801`).
- Gap:
  - API parity exists, but iOS does not reuse Android’s streaming/file-handle strategy.
- Impact:
  - Slower rechecks, extra allocations, and more file-open overhead on multi-file torrents.
- Recommended fix:
  - Port Android’s streaming verifier structure to iOS on top of a pooled file-manager layer.
  - Keep per-piece materialization as fallback, not the primary path.

### 8. iOS TCP close/error delivery can race ahead of queued data

- Classification: partial port / correctness risk
- Severity: High
- Android implementation:
  - On TCP close, Android posts back to the JS thread, drains pending TCP data first, then dispatches error/close callbacks so data ordering is preserved (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/TcpBindings.kt:399`).
- iOS implementation:
  - Incoming data is buffered in `pendingTCPFrames` until `flushTCP()` runs (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/SocketBindings.swift:658`, `ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/SocketBindings.swift:820`).
  - Connection teardown immediately calls the close callback and does not flush buffered frames first (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/SocketBindings.swift:883`).
- Gap:
  - Android carried a deliberate flush-before-close ordering fix; iOS did not.
- Impact:
  - Tail data can arrive after close or be observed in the wrong order by the JS socket layer.
  - This is a latent protocol-correctness issue, not just a perf issue.
- Recommended fix:
  - Mirror Android’s close path: drain pending TCP frames on the JS queue before dispatching close/error for that socket.

### 9. Download-root management is live on Android but disruptive and permissive on iOS

- Classification: partial port
- Severity: High
- Android implementation:
  - Roots are resolved dynamically from `RootStore`, pushed into JS via `ConfigBridge.syncRoots()`, and refreshed without rebuilding the engine (`android/app/src/main/java/com/jstorrent/app/JSTorrentApplication.kt:398`, `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/ConfigBridge.kt:289`, `android/app/src/main/java/com/jstorrent/app/NativeStandaloneActivity.kt:244`).
  - Unknown roots surface as invalid/no root rather than silently remapping (`android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/FileBindings.kt:498`, `packages/engine/src/storage/storage-root-manager.ts:79`).
- iOS implementation:
  - Changing location triggers a full controller shutdown/rebuild (`ios/JSTorrent/App/AppModel.swift:20`, `ios/JSTorrent/App/AppModel.swift:33`).
  - `FileBindings.resolveRootURL()` silently maps an unknown non-default root key to `baseDirectory/<rootKey>` instead of failing (`ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift:903`).
  - Root access is resolved once from bookmark/path state in `AppSettings`, not via a live root sync channel (`ios/JSTorrent/App/AppSettings.swift:233`, `ios/JSTorrent/App/AppSettings.swift:372`).
- Gap:
  - Android has a live runtime root-management architecture; iOS replaces the controller and hides missing-root errors behind fallback paths.
- Impact:
  - Root changes are heavier than necessary.
  - A stale/missing root key can redirect writes into the sandbox rather than preserving the Android error semantics.
- Recommended fix:
  - Add live root sync commands/config updates on iOS instead of rebuilding the controller.
  - Make unknown roots fail fast instead of synthesizing fallback directories.

### 10. Background shutdown behavior is probably justified, but iOS does not compensate for it

- Classification: justified platform divergence
- Severity: Low
- Android implementation:
  - Android differentiates between suspend, shutdown, and background restart based on policy, cache state, and foreground service state (`android/app/src/main/java/com/jstorrent/app/service/ServiceLifecycleManager.kt:179`, `android/app/src/main/java/com/jstorrent/app/JSTorrentApplication.kt:249`).
- iOS implementation:
  - iOS unconditionally shuts the runtime down when the app enters `.background` (`ios/JSTorrent/App/ContentView.swift:43`).
- Gap:
  - This is likely intentional platform divergence rather than an omitted optimization.
- Impact:
  - No background continuation path, which is acceptable on iOS only if foreground restore/cache behavior is strong.
  - Today it compounds Finding 1 because returning to foreground often means a cold runtime with no cached UI state.
- Recommended fix:
  - Keep the shutdown policy if desired, but pair it with cached summary restore and cheaper restart mechanics.

## Prioritized Action List

1. Replace iOS `StorageBindings` with a SQLite-backed KV store and build an iOS torrent-summary cache on top of it.
2. Port Android’s file-manager architecture to iOS: pooled handles, positioned I/O, validation/eviction, and no per-write `synchronize()`.
3. Fix iOS verified-write execution so batched writes run on a bounded concurrent worker pool instead of a single serial queue.
4. Remove iOS query polling (`scheduleTorrentRefreshes()` and tick-loop refreshes) once subscription replay/ref-count infrastructure exists.
5. Port Android’s callback-based promise bridge to iOS and eliminate the polling `awaitPromise()` loop.
6. Fix iOS TCP close ordering by flushing pending data before close/error callbacks.
7. Make iOS root changes live-updatable and reject unknown roots instead of silently mapping them into the sandbox.
8. Rework iOS `verifyChunks()` to stream across pooled handles instead of reopening files and materializing chunk buffers.
9. If unconditional background shutdown stays, add a first-class cached-UI restore path so it does not feel like a cold boot on every foreground return.
