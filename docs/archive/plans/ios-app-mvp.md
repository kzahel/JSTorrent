# iOS App MVP Plan

See also: [iOS Native Bindings Implementation Plan](/Users/kgraehl/code/jstorrent/docs/plans/ios-native-bindings-implementation-plan.md) for the concrete contract-first, XCTest-first bring-up plan.

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| JS Engine | **JavaScriptCore** | Built-in, ~2x faster than QuickJS, zero binary size cost. Runs in interpreter mode (no JIT for 3rd party apps) but still fastest option. |
| Background | **Foreground-only (MVP)** | iOS has no equivalent to Android's foreground service. Downloads run while app is on screen, pause when backgrounded. Location hack can be added later for AltStore PAL. |
| UI | **SwiftUI** | Minimal native shell. Thin wrapper around engine state. |
| Distribution | **AltStore PAL (EU)** / sideloading | App Store would reject a torrent client. |
| Engine bundle | **Same `engine.native.js` family** | Reuse the shared native bundle path, with small native-init adjustments for iOS-specific runtime config. |

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│  SwiftUI App (thin shell)                       │
│  - Torrent list, add torrent, progress, settings│
└──────────────┬──────────────────────────────────┘
               │ ObservableObject / state updates
┌──────────────┴──────────────────────────────────┐
│  EngineController (Swift)                       │
│  - Loads engine.native.js into JSContext         │
│  - Manages engine lifecycle                     │
│  - Bridges state updates to SwiftUI             │
└──────────────┬──────────────────────────────────┘
               │ JSContext FFI calls
┌──────────────┴──────────────────────────────────┐
│  Native Bindings (Swift)                        │
│  - FileBindings  → FileManager API              │
│  - TcpBindings   → Network.framework NWConnection│
│  - UdpBindings   → Network.framework NWConnection│
│  - StorageBindings → UserDefaults               │
│  - HashBindings  → CryptoKit / CommonCrypto     │
│  - PolyfillBindings → TextEncoder, timers, etc. │
└──────────────┬──────────────────────────────────┘
               │
┌──────────────┴──────────────────────────────────┐
│  JavaScriptCore (iOS built-in)                  │
│  - Runs engine.native.js                        │
│  - Interpreter-only (no JIT for 3rd party)      │
│  - JSContext with registered __jstorrent_* fns  │
└─────────────────────────────────────────────────┘
```

## Project Structure

```
ios/
├── JSTorrent.xcodeproj/
├── JSTorrent/
│   ├── App/
│   │   ├── JSTorrentApp.swift          # @main entry point
│   │   └── ContentView.swift           # Root SwiftUI view
│   ├── Engine/
│   │   ├── JSEngine.swift              # JSContext wrapper (like QuickJsContext.kt)
│   │   ├── JSThread.swift              # Dedicated JS thread with run loop
│   │   ├── EngineController.swift      # High-level engine API (like EngineController.kt)
│   │   └── EngineBundle.swift          # Bundle loader
│   ├── Bindings/
│   │   ├── FileBindings.swift          # __jstorrent_file_* → FileManager
│   │   ├── TcpBindings.swift           # __jstorrent_tcp_* → NWConnection
│   │   ├── UdpBindings.swift           # __jstorrent_udp_* → NWConnection (UDP)
│   │   ├── StorageBindings.swift       # __jstorrent_storage_* → UserDefaults
│   │   ├── HashBindings.swift          # __jstorrent_sha1* → CommonCrypto
│   │   ├── PolyfillBindings.swift      # TextEncoder, timers, console, crypto.random
│   │   └── NativeBindings.swift        # Facade that registers all bindings
│   ├── Views/
│   │   ├── TorrentListView.swift       # Main list of torrents
│   │   ├── AddTorrentView.swift        # Magnet link / .torrent input
│   │   └── TorrentDetailView.swift     # Single torrent progress/files
│   └── Resources/
│       └── engine.bundle.js            # Built from packages/engine/
├── JSTorrentTests/
└── Info.plist
```

## Implementation Phases

### Phase 1: Xcode Project Scaffolding
- Create Xcode project at `ios/` in the monorepo
- Minimum deployment target: iOS 16.0 (covers ~95% of devices, gives us modern SwiftUI + Network.framework)
- Add `engine.bundle.js` to bundle resources (copy from `packages/engine/dist/engine.native.js`)
- Basic SwiftUI app with placeholder UI

### Phase 2: JavaScriptCore Bridge (`JSEngine.swift`)
The core bridge between Swift and the JS engine. Key differences from Android's QuickJS bridge:

**JSC advantages over QuickJS JNI:**
- Native `JSValue` types (no string coercion bug!)
- Automatic memory management via `JSManagedValue`
- Block-based callbacks (no JNI ceremony)
- `ArrayBuffer` support via `JSObjectMakeTypedArrayWithBytesNoCopy`

**Implementation:**
```swift
class JSEngine {
    let context: JSContext
    private let jsQueue: DispatchQueue  // Serial queue = dedicated JS thread

    func evaluate(_ script: String, filename: String) -> JSValue?
    func setGlobalFunction(_ name: String, callback: @escaping ([JSValue]) -> Any?)
    func setGlobalFunctionWithBinary(_ name: String, callback: @escaping ([JSValue], Data?) -> Any?)
    func callGlobalFunction(_ name: String, args: [Any]) -> JSValue?
}
```

**Threading model:** Use a serial `DispatchQueue` instead of Android's `Handler`/`Looper`. Same semantics — all JS execution on one thread, I/O callbacks posted back to JS queue.

### Phase 3: Polyfill Bindings
Implement the polyfills that the engine bundle expects. JSC has more built-in than QuickJS, so some may not be needed:

| Polyfill | JSC has it? | Implementation |
|----------|-------------|----------------|
| `TextEncoder`/`TextDecoder` | No (JSC is not a browser) | Swift `String.Encoding.utf8` |
| `setTimeout`/`setInterval` | No | `DispatchQueue.asyncAfter` on JS queue |
| `console.log` | Partial (JSC has basic console) | Route to `os_log` / `NSLog` |
| `crypto.getRandomValues` | No | `SecRandomCopyBytes` |
| `btoa`/`atob` | No | Already pure JS polyfill in bundle |
| `URL` | No | Already polyfilled in bundle |

### Phase 4: Storage Bindings
Simplest binding — direct mapping:

| JS Function | Swift Implementation |
|-------------|---------------------|
| `__jstorrent_storage_get(key)` | `UserDefaults.standard.string(forKey:)` |
| `__jstorrent_storage_set(key, value)` | `UserDefaults.standard.set(_:forKey:)` |
| `__jstorrent_storage_delete(key)` | `UserDefaults.standard.removeObject(forKey:)` |
| `__jstorrent_storage_keys(prefix)` | Filter `UserDefaults.standard.dictionaryRepresentation().keys` |

### Phase 5: File Bindings
Map to iOS filesystem APIs:

| JS Function | Swift Implementation |
|-------------|---------------------|
| `__jstorrent_file_read(root, path, offset, len)` | `FileHandle.read(contentsOf:)` with seek |
| `__jstorrent_file_write(root, path, offset, data)` | `FileHandle.write(_:)` with seek |
| `__jstorrent_file_stat(root, path)` | `FileManager.attributesOfItem(atPath:)` |
| `__jstorrent_file_mkdir(root, path)` | `FileManager.createDirectory(atPath:withIntermediateDirectories:)` |
| `__jstorrent_file_exists(root, path)` | `FileManager.fileExists(atPath:)` |
| `__jstorrent_file_delete(root, path)` | `FileManager.removeItem(atPath:)` |
| `__jstorrent_file_readdir(root, path)` | `FileManager.contentsOfDirectory(atPath:)` |
| `__jstorrent_file_list_tree(root, path)` | Recursive directory enumeration |
| `__jstorrent_file_write_verified(...)` | Background queue: SHA1 verify then write |
| `__jstorrent_file_verify_chunks(...)` | Background queue: read files, hash chunks |

**Storage location:** App's Documents directory. `rootKey` maps to a subdirectory.

### Phase 6: Hash Bindings
| JS Function | Swift Implementation |
|-------------|---------------------|
| `__jstorrent_sha1(data)` | `CC_SHA1` from CommonCrypto (sync, on JS thread) |
| `__jstorrent_sha1_async(data, callbackId)` | Background queue → CC_SHA1 → post result to JS queue |
| `__jstorrent_sha1_batch_sync(packed)` | Loop CC_SHA1 over packed entries |

### Phase 7: TCP Bindings (Most Complex)
Use Apple's `Network.framework` (`NWConnection`):

```swift
// TCP connect
__jstorrent_tcp_connect(socketId, host, port) {
    let connection = NWConnection(host: .init(host), port: .init(rawValue: UInt16(port))!, using: .tcp)
    connections[socketId] = connection

    connection.stateUpdateHandler = { state in
        switch state {
        case .ready:
            self.jsQueue.async { /* call __jstorrent_tcp_on_connected */ }
        case .failed(let error):
            self.jsQueue.async { /* call __jstorrent_tcp_on_error */ }
        }
    }

    connection.start(queue: networkQueue)
    receiveLoop(socketId, connection)
}

// Continuous receive
func receiveLoop(_ socketId: Int, _ conn: NWConnection) {
    conn.receive(minimumIncompleteLength: 1, maximumLength: 65536) { data, _, _, error in
        if let data = data {
            // Queue data for batch dispatch (same pattern as Android)
            self.pendingTcpData.append((socketId, data))
        }
        // Continue receiving
        self.receiveLoop(socketId, conn)
    }
}
```

**Batching pattern:** Same as Android — queue incoming data on network thread, drain + pack at tick boundary via `__jstorrent_tcp_flush`.

**TLS support:** `NWConnection` with `NWProtocolTLS.Options` for `__jstorrent_tcp_secure`.

### Phase 8: UDP Bindings
Similar to TCP but with `NWConnection` using `.udp`:

```swift
let connection = NWConnection(host: host, port: port, using: .udp)
```

Or use `NWListener` for binding to a local port. Same batch pattern for incoming messages.

### Phase 9: Engine Controller + SwiftUI Integration

```swift
@MainActor
class EngineController: ObservableObject {
    @Published var torrents: [TorrentInfo] = []
    @Published var isReady = false

    private let engine: JSEngine
    private let bindings: NativeBindings

    func loadEngine() {
        // 1. Load engine.bundle.js from app bundle
        // 2. Register all __jstorrent_* bindings
        // 3. Call jstorrent.init({...})
        // 4. Set up subscription for state updates
        // 5. Start host-driven tick loop
    }

    func addTorrent(magnet: String) { ... }
    func removeTorrent(hash: String) { ... }
    func pauseTorrent(hash: String) { ... }
    func resumeTorrent(hash: String) { ... }
}
```

**State flow:** JS engine → `__jstorrent_on_state_update` callback → decode JSON → update `@Published` properties → SwiftUI re-renders.

### Phase 10: Minimal SwiftUI Views
- **TorrentListView**: List of torrents with name, progress bar, speed, status
- **AddTorrentView**: Text field for magnet link, paste from clipboard
- **TorrentDetailView**: File list, peer list, tracker info

## Key Differences from Android Implementation

| Aspect | Android | iOS |
|--------|---------|-----|
| JS Engine | QuickJS (C via JNI) | JavaScriptCore (built-in framework) |
| JS Thread | `Thread` + `Handler`/`Looper` | Serial `DispatchQueue` |
| Binary data | `ByteArray` via JNI | `JSObjectMakeTypedArray` / `Data` |
| Boolean coercion | Bug: `"false"` is truthy | No bug: JSC returns native `Bool` |
| TCP/UDP | Java NIO / Netty | Network.framework `NWConnection` |
| File I/O | SAF / `FileManager` (Kotlin) | `FileManager` / `FileHandle` (Foundation) |
| Storage | SharedPreferences | UserDefaults |
| Hashing | `MessageDigest` | CommonCrypto `CC_SHA1` |
| Background | Foreground service (legitimate) | No equivalent (foreground-only for MVP) |
| UI | Jetpack Compose | SwiftUI |

## Build Integration

Add to monorepo:
1. `ios/` directory with Xcode project
2. Build script copies `packages/engine/dist/engine.native.js` → `ios/JSTorrent/Resources/engine.bundle.js`
3. Future: `scripts/release-ios.sh` for IPA export

No CocoaPods/SPM dependencies needed for MVP — everything uses Apple frameworks:
- `JavaScriptCore.framework` (JS engine)
- `Network.framework` (TCP/UDP)
- `Foundation` (FileManager, UserDefaults)
- `CommonCrypto` (SHA1)
- `Security` (SecRandomCopyBytes)

## What This Gets You (MVP Scope)

**Working:**
- Add torrents via magnet link
- Download torrent data to app Documents directory
- See progress, speed, peer count in SwiftUI UI
- Pause/resume torrents
- Session persistence (survives app restart)
- DHT, PEX, tracker announce

**Not in MVP (but planned):**
- Torrent sonification / background audio (see below)
- .torrent file import (magnet only for MVP)
- Share extension / URL scheme handling
- Files app integration (downloaded files visible in Files)
- Settings UI
- Push to AltStore PAL

## Verification Plan

1. Build engine bundle: `cd packages/engine && pnpm bundle:native`
2. Build Xcode project, run in iOS Simulator
3. Add a test magnet link (use `pnpm seed-for-test` on dev machine for a local test torrent)
4. Verify: torrent metadata resolves, peers connect, pieces download, progress updates in UI
5. Verify: app backgrounded → downloads pause; app foregrounded → downloads resume
6. Verify: force-quit + relaunch → session restored, torrents still present

## Torrent Sonification (Post-MVP Feature)

8-bit generative audio driven by real-time torrent activity. Doubles as the background keep-alive mechanism (iOS `audio` background mode) — but it's a genuine feature, not a hack.

**Sound events:**
- **Piece verified**: 8-bit coin pickup bleep (pitch varies by piece index for variety)
- **Peer connected**: Short chirp / ascending tone
- **Peer disconnected**: Descending tone
- **Torrent complete**: Level-clear jingle
- **Hash failure**: Error buzz

**Continuous audio:**
- Background arpeggio whose tempo tracks download speed
- More peers = more voices / richer texture
- Idle = sparse ambient tones, fast download = dense chiptune

**Implementation:**
- `AVAudioEngine` with programmatic synthesis (square/saw/triangle waves) — no audio asset files needed
- Engine events feed into audio engine via the existing `__jstorrent_on_state_update` subscription
- Volume slider in settings, default at audible level. User can turn down to near-zero for silent background downloads
- `AVAudioSession` category `.playback` + `.mixWithOthers` keeps the app alive when backgrounded
- On by default. Turning volume to zero still maintains the audio session (background stays alive)

**Why this works for background downloads:**
iOS keeps apps alive that have an active audio session in `.playback` mode. Since the audio is a real feature (on at audible volume by default), the background mode usage is legitimate. The user can choose to turn volume down — that's their preference, not a hack.

## References

- **[iTorrent implementation notes](ios-reference-itorrent.md)** — Analysis of the most popular open-source iOS torrent client. Covers their background execution strategy (silent audio + location hacks), architecture, and libtorrent integration. Source cloned to `~/code/reference/iTorrent`.
