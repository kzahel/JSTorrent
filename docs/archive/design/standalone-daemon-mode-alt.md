# Standalone Daemon Mode — FSAA Alternative

**Status:** Design / brainstorming
**Date:** 2026-03-03
**See also:** [standalone-daemon-mode.md](standalone-daemon-mode.md) (daemon-owns-files approach)

## Overview

An alternative architecture for standalone/Crostini mode where the browser handles file I/O directly via the **File System Access API (FSAA)**, and the daemon is reduced to a network-only proxy. Instead of the daemon managing storage roots, reading/writing files, and verifying hashes, the engine does all of that in-process using browser-native filesystem access.

The daemon becomes a thin TCP/UDP socket multiplexer — nothing more.

## Key insight: FSAA is the Chrome Apps successor

The File System Access API is the standardized successor to `chrome.fileSystem`:

| Chrome Apps | File System Access API |
|---|---|
| `chrome.fileSystem.chooseEntry()` | `showDirectoryPicker()` |
| `chrome.fileSystem.retainEntry()` | Store `FileSystemDirectoryHandle` in IndexedDB |
| `chrome.fileSystem.restoreEntry()` | Retrieve handle from IndexedDB, call `requestPermission()` |
| `chrome.fileSystem.getWritableEntry()` | `handle.createWritable()` |
| `DirectoryEntry.createReader()` | `dirHandle.values()` / async iteration |

**Critical difference for extensions:** Chrome extensions get automatic re-grant of permissions on restored handles — no user gesture needed. The extension can restore a `FileSystemDirectoryHandle` from IndexedDB on startup and immediately use it. Website contexts require a user gesture (click) to re-grant, but the prompt is just "Allow access to **Downloads**?" — the user doesn't have to re-navigate a file picker.

## Architecture comparison

```
Main doc (daemon owns files):

  Extension ──HTTP──▶ Daemon ──▶ { files, TCP/UDP }
                       ├── /read, /write, /ops/...
                       ├── /browse, /roots
                       └── /io (WebSocket: TCP/UDP + control)

This doc (FSAA, browser owns files):

  Extension ──FSAA──▶ filesystem (direct, no round-trip)
  Extension ──WS────▶ Daemon ──▶ { TCP/UDP only }
                       └── /io (WebSocket: TCP/UDP + control)
```

### What the daemon loses

All file-related endpoints are eliminated:

- `/read`, `/write` — replaced by `FileSystemFileHandle` read/write
- `/ops/stat`, `/ops/exists`, `/ops/list`, `/ops/list_tree` — replaced by FSAA directory/file operations
- `/ops/delete`, `/ops/batch_delete` — replaced by `FileSystemDirectoryHandle.removeEntry()`
- `/ops/verify_chunks` — replaced by in-browser SHA1 hashing (engine already has the piece data)
- `/browse` — replaced by `showDirectoryPicker()` or in-app FSAA-based folder browser
- `/roots` (POST) — roots are `FileSystemDirectoryHandle` instances stored in IndexedDB
- Corresponding Rust code in `desktop/io-daemon/src/files.rs` — not needed

### What the daemon keeps

- WebSocket at `/io` — TCP/UDP socket multiplexing only
- `/status`, `/health` — version, connectivity checks
- `/self-update` — binary self-replacement (standalone mode)
- Auth handshake — token-based authentication
- Config persistence — auth token only (no roots config needed)

### Deployment matrix (updated)

| Platform | Setup | Engine runs in | File I/O | Network I/O |
|----------|-------|---------------|----------|-------------|
| Mac/Win/Linux (extension) | Extension + Desktop app | Extension UI page | **FSAA (direct)** | Rust io-daemon (via native host) |
| Mac/Win/Linux (standalone) | **io-daemon binary only** | **Website** | **FSAA (direct)** | **Rust io-daemon** |
| ChromeOS (Crostini) | Extension + io-daemon | Extension UI page | **FSAA (direct)** | Rust io-daemon (standalone) |
| ChromeOS (Crostini, no ext) | **io-daemon binary only** | **Website** | **FSAA (direct)** | **Rust io-daemon** |
| ChromeOS (ext + Android) | Extension + Android app | Extension UI page | **FSAA (direct)** | Android companion |
| Android phone | Android app | QuickJS (in-process) | NativeFileSystem (JNI) | NativeSocketFactory |

Note: Android standalone mode is unaffected — QuickJS doesn't have FSAA, so the `NativeFileSystem` JNI adapter remains as-is.

## New adapter: `FsaaFileSystem`

A new `IFileSystem` implementation that wraps the File System Access API. This is the only significant engine code change — everything else is removal of daemon file endpoints.

### Interface mapping

Every `IFileSystem` / `IFileHandle` method maps cleanly to FSAA:

```
IFileSystem method          FSAA equivalent
─────────────────────────   ─────────────────────────────────────────
open(path, mode)            dirHandle.getFileHandle(name, {create: mode !== 'r'})
stat(path)                  handle.getFile() → {size, lastModified}
mkdir(path)                 dirHandle.getDirectoryHandle(name, {create: true})
exists(path)                try getFileHandle/getDirectoryHandle, catch → false
readdir(path)               dirHandle.values() → async iteration
delete(path)                parentHandle.removeEntry(name, {recursive: true})
listTree(path)              recursive dirHandle.values() + getFile() for sizes
batchDelete(dir, entries)   loop: dirHandle.removeEntry(name), collect failures
verifyChunks(request)       read files via getFile().slice(), hash in JS
```

### `IFileHandle` mapping

```
IFileHandle method          FSAA equivalent
─────────────────────────   ─────────────────────────────────────────
read(buf, off, len, pos)    handle.getFile().slice(pos, pos+len) → arrayBuffer()
write(buf, off, len, pos)   handle.createWritable() → seek(pos), write(subarray)
truncate(len)               writable.truncate(len)
sync()                      writable.close() (FSAA flushes on close)
close()                     release handle reference
```

### Path resolution

FSAA works with handle hierarchies, not string paths. The adapter needs to resolve `"path/to/file.dat"` by walking directory handles:

```typescript
class FsaaFileSystem implements IFileSystem {
  constructor(private root: FileSystemDirectoryHandle) {}

  private async resolve(path: string): Promise<{
    parent: FileSystemDirectoryHandle
    name: string
  }> {
    const parts = path.split('/').filter(Boolean)
    const name = parts.pop()!
    let dir = this.root
    for (const part of parts) {
      dir = await dir.getDirectoryHandle(part)
    }
    return { parent: dir, name }
  }
}
```

### `verifyChunks` — no daemon needed

The daemon approach sends piece hashes to the daemon, which reads files from disk, hashes them, and returns match/mismatch results. With FSAA, the engine does this entirely in-browser:

1. Read file data via `FileSystemFileHandle.getFile().slice()`
2. Hash each chunk with `crypto.subtle.digest('SHA-1', chunk)` (native browser crypto, fast)
3. Compare against expected hashes
4. Return result array

This is the same logic already implemented in `InMemoryFileSystem.verifyChunks()` — it can be extracted into a shared utility that both `InMemoryFileSystem` and `FsaaFileSystem` use.

**Performance note:** `crypto.subtle.digest` runs on the browser's native crypto thread. For large verifications, this should be comparable to the daemon's Rust SHA1 implementation, minus the HTTP round-trip overhead.

### Write performance

The daemon approach: piece data → serialize to HTTP body → send to daemon → daemon writes to disk.

FSAA approach: piece data → `FileSystemWritableFileStream.write()` → browser writes to disk.

FSAA skips the HTTP serialization/deserialization round-trip. For sustained torrent downloads at tens of MB/s, the eliminated overhead could be meaningful. The browser's I/O path goes through its own sandbox layer, but for sequential writes this should be efficient.

## Storage root management

### Extension context (automatic re-grant)

```typescript
// First time: user picks a download directory
const dirHandle = await showDirectoryPicker({ mode: 'readwrite' })

// Persist to IndexedDB
const db = await openDB('jstorrent-roots')
await db.put('roots', dirHandle, rootKey)

// On startup: restore handle — permission is automatic for extensions
const handle = await db.get('roots', rootKey)
// No requestPermission() call needed — extension has persistent access
const fs = new FsaaFileSystem(handle)
```

### Website context (one-click re-grant)

```typescript
// On startup: restore handle, request permission with user gesture
const handle = await db.get('roots', rootKey)

// Show "Resume downloads" button, on click:
const perm = await handle.requestPermission({ mode: 'readwrite' })
if (perm === 'granted') {
  const fs = new FsaaFileSystem(handle)
}
```

The permission prompt is a simple browser bar: "Allow jstorrent.com to edit files in **Downloads**?" — the user clicks Allow. No file picker navigation required.

### Integration with `StorageRootManager`

The existing `StorageRootManager` pattern works unchanged. The factory function just creates `FsaaFileSystem` instances instead of `DaemonFileSystem`:

```typescript
// Current daemon preset:
const storageRootManager = new StorageRootManager((root) => {
  return new DaemonFileSystem(connection, root.key)
})

// FSAA preset:
const storageRootManager = new StorageRootManager((root) => {
  const dirHandle = handleCache.get(root.key)  // from IndexedDB
  return new FsaaFileSystem(dirHandle)
})
```

### Folder browser UX

Two options, not mutually exclusive:

1. **Native picker** — `showDirectoryPicker()` opens the OS folder picker. Clean, familiar, zero UI code. Works everywhere FSAA is supported.

2. **In-app browser** — If more control is needed (showing free space, filtering), the `FsaaFileSystem.readdir()` method can power a custom folder browser modal, navigating the already-granted root's subtree. But for picking *new* roots, `showDirectoryPicker()` is the right UX.

For Crostini specifically, `showDirectoryPicker()` can navigate to `/mnt/chromeos/removable/` for USB drives, just as the daemon's `GET /browse` would.

## What this eliminates from the main doc

Features from `standalone-daemon-mode.md` that become unnecessary:

| Feature | Main doc | This alternative |
|---------|----------|-----------------|
| `GET /browse` endpoint | Daemon reads directory listings | FSAA `readdir()` / `showDirectoryPicker()` |
| `POST /roots` endpoint | Daemon persists roots to config | Roots are IndexedDB-stored handles |
| `~/.config/jstorrent-standalone/config.json` roots | Daemon-side root config | IndexedDB in browser (daemon only stores auth token) |
| Folder browser modal | Custom UI calling daemon `/browse` | `showDirectoryPicker()` or FSAA-powered browser |
| `--download-root` flag | Install script bakes path into systemd | User picks folder on first run via picker |
| All `/read`, `/write`, `/ops/*` endpoints | Daemon file I/O | Direct FSAA I/O |
| `desktop/io-daemon/src/files.rs` | ~500+ lines of Rust file handling | Not needed |

Features that remain unchanged:
- Self-update (`POST /self-update`)
- Metrics check-in (24h cadence)
- WebSocket TCP/UDP multiplexing
- Auth handshake
- Install script (simplified — no `--download-root`)
- Systemd service management

## New "IFileSystem method" checklist

Compare with the 7-step checklist in CLAUDE.md. With FSAA handling file I/O in-browser, adding a new `IFileSystem` method requires fewer backend implementations:

1. **Interface**: `packages/engine/src/interfaces/filesystem.ts`
2. **TS adapters (5)**: `node/`, `scoped-node/`, `native/`, `memory/`, `null/` — plus new `fsaa/`
3. **Android FileManager**: Still needed for Android standalone
4. **Android FileBindings**: Still needed for QuickJS JNI

**Eliminated:**
- ~~Android companion HTTP endpoint~~ (if Android extension path also uses FSAA)
- ~~Rust io-daemon endpoint~~ (daemon no longer handles files)

That's 2 fewer implementations per method, and the eliminated ones are the cross-language ones (Kotlin HTTP handler, Rust HTTP handler) — the most friction-heavy steps.

## Open questions

- [ ] **FSAA in Crostini Chrome**: Does `showDirectoryPicker()` work correctly inside Chrome on Crostini? Can it see `/mnt/chromeos/removable/` for USB drives? Need to test.
- [ ] **Extension FSAA permissions**: Confirm that MV3 extensions get automatic re-grant on restored `FileSystemDirectoryHandle` from IndexedDB without a user gesture. (This is the documented behavior but worth verifying.)
- [ ] **Write performance**: Benchmark FSAA `FileSystemWritableFileStream` write throughput vs HTTP-to-daemon writes for sustained large file downloads. Is the browser's sandbox I/O path fast enough?
- [ ] **`createWritable()` semantics**: Each `createWritable()` call creates a new writable stream. For random-access writes (torrent piece placement), need to verify that `seek()` + `write()` + `close()` on a writable stream does an in-place update, not a copy-on-write-then-rename. The `keepExistingData: true` option on `createWritable()` is relevant here.
- [ ] **Concurrent writes**: Can multiple `FileSystemWritableFileStream` instances write to the same file concurrently (different offsets)? Or does the API serialize? This matters for disk queue batching.
- [ ] **Large file support**: FSAA's `getFile()` returns a `File` (Blob). For multi-GB torrent files, `slice()` should work without loading the whole file into memory, but worth confirming.
- [ ] **`verifyChunks` performance**: Is `crypto.subtle.digest('SHA-1')` fast enough for bulk verification? Compare against the daemon's Rust SHA1 (which uses hardware acceleration on modern CPUs). Could also use a WASM SHA1 if needed.
- [ ] **Hybrid approach**: Could the extension use FSAA for file I/O but fall back to daemon file endpoints when FSAA is unavailable (older browsers, restricted contexts)? Or is it cleaner to pick one path per deployment?
- [ ] **Android companion simplification**: If the extension uses FSAA on ChromeOS, the Android companion only needs to provide TCP/UDP — its file endpoints (`FileManager`, `NettyHttpServer` file routes) could be removed for that deployment path. But the companion still needs file I/O for Android standalone mode.

## Implementation order

1. **`FsaaFileSystem` + `FsaaFileHandle`** — new adapter in `packages/engine/src/adapters/fsaa/`. Start with `open`, `read`, `write`, `stat`, `mkdir`, `exists`, `readdir`.
2. **`verifyChunks`** — extract shared SHA1 verification logic from `InMemoryFileSystem`, reuse in `FsaaFileSystem`.
3. **`listTree`, `delete`, `batchDelete`** — remaining `IFileSystem` methods.
4. **Root persistence** — IndexedDB store for `FileSystemDirectoryHandle` instances, restore on startup.
5. **New preset** — `createFsaaEngine()` or extend `createDaemonEngine()` to accept an FSAA filesystem factory instead of daemon connection for file I/O.
6. **Integration test** — download a torrent using FSAA file I/O + daemon networking, verify data integrity.
7. **Strip daemon file endpoints** — behind a feature flag initially, then remove when FSAA path is proven.

## Risks

- **Browser API stability**: FSAA is relatively new. The core API (`showDirectoryPicker`, `getFileHandle`, `createWritable`) is stable in Chrome, but edge cases around `keepExistingData`, concurrent streams, and permission re-grant in extensions may have subtle bugs.
- **Not available everywhere**: FSAA is Chrome/Edge only. Firefox and Safari don't support it. For JSTorrent this is fine (Chrome extension), but limits the website standalone path to Chromium browsers.
- **`createWritable` performance model**: If each `write()` requires opening a new writable stream (which does a temp-file-then-rename internally), random-access write performance could be poor. The `keepExistingData: true` option mitigates this, but the exact I/O pattern needs benchmarking.
