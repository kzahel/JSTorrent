# Boundary Piece Write Latency — Android Standalone (and iOS)

**Date:** 2025-03-15
**Observed on:** Android standalone, torrent with 32 MB pieces (~1 GB episodes, some skipped)
**Symptom:** JS thread latency spikes to 23–30 seconds during boundary piece writes. Download speed drops to ~200 KB/s during stalls, recovers to 4.5+ MB/s between them.

## Background

Pieces are classified by what files they touch:

- **`wanted`** (single-file): fits within one selected file → fast verified-write path (hash + write batched to Kotlin I/O threads via `NativeBatchingDiskQueue`)
- **`wanted`** (multi-file, no skips): spans two selected files → manual hash verify + sync write per file
- **`boundary`** (with skips): spans selected + skipped files → hash verify + `.parts` file write (full piece) + filtered write (wanted portions only)

With 32 MB pieces and ~1 GB files, most pieces are single-file. At each file boundary there's one boundary piece. With 10 episodes and some skipped, there are roughly 5–10 boundary pieces per torrent.

## Platform Comparison

### Single-file pieces (the fast path)

| Platform | Mechanism | Blocks JS? |
|---|---|---|
| Android standalone | `NativeBatchingDiskQueue` → verified batch FFI → Kotlin I/O threads | No |
| Extension + Desktop | `DaemonFileHandle` with `X-Expected-SHA1` → HTTP to io-daemon | No |
| Extension + Companion | Same, HTTP to Android companion server | No |
| iOS | Same as Android standalone | No |
| Node CLI | `NodeHasher.sha1()` (sync) + `fs.promises.write()` | Yes (hash only) |

### Boundary pieces WITHOUT skips (classification=`wanted`, multi-file)

`writePieceVerified()` → `pieceSpansSingleFile()` returns null → falls back to `this.write()` (unverified multi-file) → caller does manual hash + write.

| Step | Android / iOS | Ext + Desktop / Companion | Node CLI |
|---|---|---|---|
| Hash verify | `NativeHasher.sha1()` → async Kotlin I/O | Web Worker / SubtleCrypto → async | **Sync `crypto.createHash`** |
| Write per file | `NativeFileHandle.write()` → **sync `__jstorrent_file_write` FFI** | `DaemonFileHandle.write()` → async HTTP | `fs.promises.write()` → async |
| Disk queue | `PassthroughDiskQueue` — immediate | `TorrentDiskQueue` — worker pool | `TorrentDiskQueue` |
| **Blocks JS?** | **Yes — sync FFI per write** | No | **Yes — hash** |

### Boundary pieces WITH skips (classification=`boundary`)

`verifyAndWriteBoundaryPiece()` → hash → drain queue → write `.parts` (full piece data) → write wanted portions to content files.

| Step | Android / iOS | Ext + Desktop / Companion | Node CLI |
|---|---|---|---|
| 1. Hash verify | `NativeHasher.sha1()` → async | Web Worker / SubtleCrypto → async | **Sync** |
| 2. `diskQueue.drain()` | Passthrough → no-op | `TorrentDiskQueue` → waits for running jobs | Waits for running jobs |
| 3. `.parts` write (full 32 MB) | **Sync `__jstorrent_file_write` FFI** | Async HTTP | `fs.promises.write()` → async |
| 4. `.parts` header write | **Sync FFI** | Async HTTP | Async |
| 5. `handle.sync()` | No-op | No-op | `fsync` → async |
| 6. Filtered file writes | Per wanted file: **sync FFI** | Per wanted file: async HTTP | Async |
| **Blocks JS?** | **Yes — .parts + filtered writes all sync** | Only drain() | **Yes — hash** |

### Summary Table

| Platform | Single-file | Boundary (no skip) | Boundary (with skip) |
|---|---|---|---|
| **Android standalone** | Async batch | **Sync FFI writes** | **Sync FFI: .parts + filtered** |
| **iOS** | Async batch | **Sync FFI writes** | **Sync FFI: .parts + filtered** |
| **Extension + Desktop** | Async HTTP | Async HTTP | Async HTTP (drain may wait) |
| **Extension + Companion** | Async HTTP | Async HTTP | Async HTTP (drain may wait) |
| **Node CLI** | Sync hash, async write | Sync hash, async write | Sync hash, async write |

## Root Cause Analysis

On Android standalone / iOS, boundary piece writes bypass `NativeBatchingDiskQueue` and call `__jstorrent_file_write()` synchronously from the JS thread. For a 32 MB boundary piece with skips:

1. **`.parts` file write** — `NativeFileHandle.write()` without `pendingHash` → sync `__jstorrent_file_write()`:
   - `ArrayBuffer.slice()` copies 32 MB in JS (line 122–123 of `native-file-handle.ts`)
   - 32 MB transferred across JNI boundary (another copy)
   - Kotlin `FileManager.writeFile()` writes through SAF
2. **`.parts` header write** — sync FFI, small data
3. **Filtered content writes** — for each wanted file touched by the piece, same sync path with another `ArrayBuffer.slice()` + JNI transfer + SAF write

Total per boundary piece: ~64–96 MB of temporary `ArrayBuffer` allocations + JNI copies + SAF writes, all blocking the JS thread.

### Why 23–30 seconds (not milliseconds)?

Raw sequential I/O for 32 MB should take < 100 ms on modern UFS storage. The extreme latency likely comes from a combination of:

- **QuickJS GC pressure**: multiple 32 MB `ArrayBuffer.slice()` allocations trigger QuickJS garbage collection (no incremental GC). QuickJS GC is stop-the-world.
- **SAF overhead**: Android's Storage Access Framework adds significant per-operation latency, especially for DocumentFile URIs vs raw file paths.
- **JNI data transfer**: copying large byte arrays across JNI is expensive (must pin + copy).
- **Memory fragmentation**: after several boundary pieces, the QuickJS heap may be fragmented, making large allocations slower.
- **Cascading effect**: while JS thread is blocked on sync writes, incoming peer data backs up in the handler queue (screenshot showed handler Q max: 51). When the thread unblocks, it has to process the backlog, potentially triggering more GC.

## Code References

- `packages/engine/src/core/torrent.ts` — `verifyAndWriteBoundaryPiece()`, `finalizePiece()`
- `packages/engine/src/core/torrent-content-storage.ts` — `writePieceVerified()`, `writePieceFilteredByPriority()`, `write()`
- `packages/engine/src/core/parts-file.ts` — `addPieceAndFlush()`, `flush()`
- `packages/engine/src/adapters/native/native-file-handle.ts` — `write()` (sync FFI path at line 136)
- `packages/engine/src/adapters/native/native-batching-disk-queue.ts` — batching (only used for verified writes)
- `packages/engine/src/adapters/daemon/daemon-file-handle.ts` — `write()` (async HTTP path)
- `packages/engine/src/core/disk-queue.ts` — `PassthroughDiskQueue`, `TorrentDiskQueue`

## Proposed Solutions

### Option A: Async native write FFI (recommended)

Add `__jstorrent_file_write_async(rootKey, path, position, data, callbackId)` to the Kotlin bindings. Same pattern as `__jstorrent_sha1_async` — dispatch to I/O thread, post result back to JS thread via callback.

**Pros:** Fully unblocks JS thread. Minimal engine-side changes (swap sync call for async in `NativeFileHandle.write()`).
**Cons:** Requires new Kotlin binding + callback dispatch plumbing.

### Option B: Route boundary writes through `NativeBatchingDiskQueue`

Currently boundary piece writes bypass the disk queue and call `handle.write()` directly. Instead, queue them like single-file verified writes.

**Pros:** Reuses existing async infrastructure. Batching already handles hash + write on Kotlin I/O threads.
**Cons:** `NativeBatchingDiskQueue` is designed for verified writes (hash + write atomic). Boundary pieces need a different pattern: hash is already verified, just need async write. Would need a "write-only" (no-hash) batch item type. Also, `.parts` file writes don't go through `TorrentContentStorage` at all — they use `IFileSystem` directly from `PartsFile`, so this wouldn't help the `.parts` path without additional refactoring.

### Option C: Eliminate the `ArrayBuffer.slice()` copy

`NativeFileHandle.write()` currently does:
```typescript
const sub = buffer.subarray(offset, offset + length)
const arrayBuffer = sub.buffer.slice(sub.byteOffset, sub.byteOffset + sub.byteLength)
```

The `.slice()` creates a 32 MB copy. If the Kotlin side can accept an offset+length with the original ArrayBuffer (or if QuickJS can transfer ownership), this eliminates one 32 MB allocation.

**Pros:** Reduces GC pressure by ~50%. Simple change.
**Cons:** Still sync. Doesn't fix the fundamental blocking problem. May require changes to `setGlobalFunction` JNI bridge to support offset/length parameters.

### Option D: Chunked sync writes with yield

Break the 32 MB write into smaller chunks (e.g., 256 KB) and `await` a microtask yield between each chunk. This keeps the JS thread responsive between chunks.

```typescript
const CHUNK_SIZE = 256 * 1024
for (let off = 0; off < length; off += CHUNK_SIZE) {
  const chunk = Math.min(CHUNK_SIZE, length - off)
  __jstorrent_file_write(rootKey, path, position + off, data.slice(off, off + chunk))
  await yieldMicrotask()
}
```

**Pros:** No Kotlin changes needed. Keeps latency bounded (~1 ms per chunk).
**Cons:** Many small FFI calls add overhead. Many small `ArrayBuffer.slice()` allocations (though much smaller). SAF may perform worse with many small writes vs one large write.

### Option E: Avoid `.parts` file for large pieces

For very large piece sizes (e.g., ≥ 8 MB), skip the `.parts` file entirely. Write the wanted portions directly and discard the unwanted portions. If the piece is later needed (user un-skips a file), re-download it.

**Pros:** Eliminates the largest single write (32 MB to `.parts`). Simpler boundary handling.
**Cons:** Wasted bandwidth if user un-skips. Changes `.parts` file semantics. Need to track which boundary pieces were discarded.

### Recommended Approach

**Short term:** Option C (eliminate the copy) + Option D (chunked writes) — reduces the worst case from ~30 s to a few hundred ms with no Kotlin changes.

**Medium term:** Option A (async write FFI) — properly fixes the problem at the architecture level. Once async writes exist, boundary pieces behave identically to the daemon/extension path.
