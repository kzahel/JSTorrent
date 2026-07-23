# Download Manifest Files — Implementation Plan

## Overview

JSTorrent writes a dot-prefixed JSON manifest file per torrent in the download directory. PlayVideo reads these during directory scan to attach torrent identity (infohash + fileIndex) to library entries, enabling stable cross-device sync.

## Sidecar Format

Filename: `.{infohash}.jstorrent.json` (full 40-char hex infohash)

```json
{
  "infohash": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
  "magnet": "magnet:?xt=urn:btih:a1b2c3d4e5...&dn=Yellowstone+S01&tr=udp://...",
  "files": {
    "Season 1/Yellowstone.S01E01.mkv": { "index": 0, "complete": true },
    "Season 1/Yellowstone.S01E02.mkv": { "index": 1, "complete": true },
    "Season 2/Yellowstone.S02E01.mkv": { "index": 2, "complete": false },
    "Season 2/Yellowstone.S02E02.mkv": { "index": 3, "complete": false }
  }
}
```

- `infohash` — redundant with filename, included for convenience
- `magnet` — full magnet with trackers, for "start this torrent on another device"
- `files` — keys are paths relative to manifest directory, `/` separator always
- `index` — fileIndex in the torrent
- `complete` — whether the file has fully downloaded

### File Placement

**Multi-file torrent** — manifest in the torrent folder:
```
~/Downloads/Yellowstone S01 1080p/
  .a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0.jstorrent.json
  Season 1/Yellowstone.S01E01.mkv
  Season 1/Yellowstone.S01E02.mkv
```

**Single-file torrent** — manifest in the downloads root (infohash prevents collisions):
```
~/Downloads/
  .f6g7h8i9j0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5.jstorrent.json
  Movie.A.mkv
```

## Setting

New boolean setting: `downloadManifest`

- Category: `setting`
- Storage: `local`
- Default: `false`
- Off by default — beta feature, opt-in

Add to `config-schema.ts` and expose via `BaseConfigHub`.

## Engine Logic

### When to Write

1. **After metadata is available** — write manifest with all files, `complete: false` for each. For .torrent adds this is immediate; for magnet adds, after metadata fetch completes.
2. **On file completion** — mark dirty, debounced flush sets `complete: true` for completed files.
3. **On torrent done** — mark dirty, flush sets all files `complete: true` (catch-all).

### Write Debouncing

Follows the same pattern as `SessionPersistence.schedulePiecePersistence()`:
- Dirty infohashes collected in a `Set<string>`
- 1-second flush timer coalesces rapid piece completions into a single write
- Emergency flush on shutdown via `flushPendingSaves()`

### When to Delete

- Always delete manifest when torrent is removed (regardless of "delete data" setting — the manifest is JSTorrent metadata, not user data).

### Where the Logic Lives

New module: `packages/engine/src/core/manifest-writer.ts`

- Pure function `buildManifestJson(torrent)` — produces the JSON object
- `ManifestWriter` class — listens to torrent events, calls `writeAtomic` / `delete` on IFileSystem
- Debounces writes using dirty-set + 1s flush timer (mirrors `SessionPersistence`)
- Instantiated by `BtEngine` when `downloadManifest` setting is enabled
- Subscribes to setting changes to start/stop

### Path Resolution

The manifest path is: `{torrent save path}/.{infohash}.jstorrent.json`

For multi-file torrents, the save path includes the torrent name folder. For single-file torrents, it's the storage root directly.

File keys in the JSON are relative to the directory containing the manifest.

## New IFileSystem Method: `writeAtomic`

```typescript
interface IFileSystem {
  // ... existing methods ...
  writeAtomic(path: string, data: Uint8Array): Promise<void>
}
```

Each backend implements atomicity using platform-native primitives. The caller doesn't manage temp files.

### Implementation per Adapter

| Adapter | Implementation |
|---------|---------------|
| **NodeFileSystem** | `fs.writeFile(tmp)` + `fs.rename(tmp, path)` with random tmp suffix |
| **ScopedNodeFileSystem** | Delegate to inner NodeFileSystem with scoped path |
| **DaemonFileSystem** | New HTTP endpoint `POST /ops/write_atomic` — daemon does tmp+rename server-side |
| **NativeFileSystem** | New JNI binding `__jstorrent_file_write_atomic` — Kotlin uses `AtomicFile` |
| **InMemoryFileSystem** | Direct write to memory map (inherently atomic) |
| **NullFileSystem** | No-op |

### Backend Changes

#### Rust io-daemon (`desktop/io-daemon/src/files.rs`)

New endpoint:
```
POST /ops/write_atomic?root_key=...&path=...
Body: raw bytes
```

Implementation: write to `{path}.{random}.tmp`, then `std::fs::rename`.

#### Android FileManager (`android/io-core/.../FileManager.kt`)

New method:
```kotlin
fun writeAtomic(rootUri: Uri, relativePath: String, data: ByteArray)
```

Implementation: `AtomicFile` wrapper or manual tmp+rename via SAF.

#### Android FileBindings (`android/quickjs-engine/.../FileBindings.kt`)

Register new JNI function: `__jstorrent_file_write_atomic(rootKey, path, data)`

#### Android Companion HTTP (`android/companion-server/.../NettyHttpServer.kt`)

New endpoint: `POST /ops/write_atomic` — mirrors daemon endpoint.

#### iOS FileBindings (`ios/JSTorrentKit/.../FileBindings.swift`)

New binding: `__jstorrent_file_write_atomic` — uses `Data.write(to:options:.atomic)`.

## Implementation Order

### Phase 1: Engine + Node (testable end-to-end)

1. Add `writeAtomic` to `IFileSystem` interface
2. Implement in `NodeFileSystem`, `ScopedNodeFileSystem`, `InMemoryFileSystem`, `NullFileSystem`
3. Add Node adapter tests (`test/adapters/node/write-atomic.test.ts`, `test/adapters/memory/write-atomic.test.ts`)
4. Add `downloadManifest` setting to `config-schema.ts` + `BaseConfigHub`
5. Implement `manifest-writer.ts` — build JSON, debounced write/update/delete
6. Wire `ManifestWriter` into `BtEngine` torrent lifecycle
7. Unit tests for `ManifestWriter` with `InMemoryFileSystem`
8. Integration test with Node adapter

### Phase 2: Daemon (extension + Tauri)

9. Add `POST /ops/write_atomic` endpoint to Rust io-daemon
10. Implement `writeAtomic` in `DaemonFileSystem`
11. Add conformance cases to `contracts/io-daemon-conformance.json` (e.g. `ops.write_atomic.creates_file`, `ops.write_atomic.overwrites_existing`)
12. Add conformance-tagged tests in `daemon-contract-conformance.test.ts` for Rust
13. Test via extension or Tauri

### Phase 3: Android (native + companion)

14. Add `writeAtomic` to `FileManager` interface + `FileManagerImpl`
15. Add JNI binding in `FileBindings.kt`
16. Add companion HTTP endpoint in `NettyHttpServer.kt`
17. Add Android conformance tests for the same `ops.write_atomic.*` case IDs
18. Implement `writeAtomic` in `NativeFileSystem`
19. Test on emulator

### Phase 4: iOS

20. Add `writeAtomic` binding in `FileBindings.swift`
21. Test on simulator

## Verification

Per the IFileSystem checklist:
```
pnpm typecheck && pnpm test          # Engine
cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace  # Desktop
./gradlew :app:compileDebugKotlin && ./gradlew testDebugUnitTest  # Android
xcodebuild -scheme JSTorrent -destination 'platform=iOS Simulator,name=iPhone 16' build  # iOS
```

## Resolved Decisions

- **Delete on torrent remove?** Yes — always delete manifest when torrent is removed. The manifest is JSTorrent metadata, not user data.
- **Atomicity?** Yes — `writeAtomic` prevents partial-write errors that would cause parse failures in playsvideo.
- **Write debouncing?** Yes — 1s coalesce timer following `SessionPersistence` pattern. Prevents excessive writes during fast downloads.
- **Setting name?** `downloadManifest` (user-facing), files remain `.{hash}.jstorrent.json`.
- **Android SAF dot-prefixed files?** Confirmed OK — SAF `DocumentFile` APIs accept dot-prefixed filenames without filtering.

## Open Questions

- **Filter to video files only?** The `files` map could include non-video files (NFOs, subtitles, etc.). PlayVideo ignores non-video entries anyway. Simpler to include everything.
- **NFC normalization?** Probably not needed now. macOS NFD decomposition could cause mismatches between manifest keys and scanned filenames. Add if it becomes a real bug.
