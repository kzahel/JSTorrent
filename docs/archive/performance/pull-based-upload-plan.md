# Pull-Based Upload System with Per-Peer Send Buffer Watermarks

## Goal
Replace the blunt "choke on backpressure" hack (`MAX_PENDING_READS`) with a pull-based upload model inspired by libtorrent. Uploads gated by per-peer send buffer capacity. Works across all backends (Android standalone, extension+daemon, Node.js daemon).

## Key Design Decisions

### Pull model: Tick-driven upload serving
Serve uploads during a new tick phase (UPLOAD). For each unchoked peer, issue disk reads only while the peer's send buffer has room. Requests that arrive during GATHER are queued, not eagerly served.

### Per-peer watermark using `sendQueueBytes`
Since we can't see kernel socket buffers, use `sendQueueBytes` (already tracked on PeerConnection) as the backpressure signal:
```
sendQueueBytes + readingBytes < watermark
```

### All reads are async on all platforms
- **Android**: New `__jstorrent_file_read_batch` async FFI. Dispatches to Kotlin `Dispatchers.IO`, results queued in `pendingDiskReadResults`, delivered at next tick via existing flush mechanism (same pattern as verified writes). **No JS thread blocking.**
- **Daemon/Extension**: Already async (HTTP GET to Rust daemon / NativeMessaging).
- Both use the same `fillSendBuffers()` call and watermark check.
- `readingBytes` stays elevated until async read completes. Watermark naturally limits outstanding reads.

### Tick-aligned sending
`peer.sendPiece()` only appends to `sendQueue`. Actual socket writes happen in Phase 4 OUTPUT `flush()`. This is true on all platforms - sends are always batched and tick-aligned.

---

## Files to Modify

### 1. `packages/engine/src/core/torrent-uploader.ts` — Major refactor

**Remove**: `drainQueue()`, `drainScheduled`, global `pendingReads`, `MAX_PENDING_READS`, `chokePeer` callback.

**New design**:
```ts
interface PeerUploadState {
  requests: QueuedUploadRequest[]  // per-peer request queue
  readingBytes: number              // bytes with outstanding disk reads
}
```

- **Per-peer state**: `Map<PeerConnection, PeerUploadState>`
- **`queueRequest()`**: Validate, add to per-peer queue. Does NOT trigger reads.
  - Per-peer request queue limit (default 500). Reject excess (no choking).
- **`fillSendBuffers(peers)`**: Called during tick UPLOAD phase.
  - For each unchoked peer with queued requests:
    - Watermark check: `peer.sendBufferBytes + state.readingBytes >= sendBufferWatermark` → skip
    - Rate limit check: `uploadBucket.tryConsume(req.length)` → stop all if rate-limited
    - Dequeue request, `state.readingBytes += req.length`
    - Issue `contentStorage.read()` → promise
    - On completion: `readingBytes -= length`, `peer.sendPiece()`, `recordUpload()`
- **Keep**: Rate limit bucket (upload speed cap separate from backpressure)

**Configurable statics** (set per-platform in presets):
| Setting | Android | Daemon/Extension | Default |
|---------|---------|-------------------|---------|
| `SEND_BUFFER_WATERMARK` | 64KB | 512KB | 512KB |
| `MAX_REQUEST_QUEUE_PER_PEER` | 500 | 500 | 500 |

### 2. `packages/engine/src/core/peer-connection.ts` — Expose sendQueueBytes

Add public getter:
```ts
get sendBufferBytes(): number { return this.sendQueueBytes }
```

### 3. `packages/engine/src/core/torrent-tick-loop.ts` — New UPLOAD phase

Insert Phase 3.5 between REQUEST and OUTPUT:
```
Phase 1: GATHER
Phase 2: PROCESS
Phase 3: REQUEST
Phase 3.5: UPLOAD  ← NEW: this.callbacks.fillSendBuffers(connectedPeers)
Phase 4: OUTPUT
```
Add `fillSendBuffers` to `TorrentTickLoopCallbacks`. Add timing metric.

### 4. `packages/engine/src/core/torrent.ts` — Wire up callback

- Add `fillSendBuffers` callback → `this._uploader.fillSendBuffers(peers)`
- Remove `chokePeer` from uploader constructor

### 5. `packages/engine/src/presets/native.ts` — Remove hack, set watermarks

- **Remove**: `TorrentUploader.MAX_PENDING_READS = 8`
- **Add**: `TorrentUploader.SEND_BUFFER_WATERMARK = 64 * 1024`

### 6. Android Kotlin: Async read batch — New Kotlin + TS code

**Kotlin** (`FileBindings.kt`):
- Add `__jstorrent_file_read_batch(packed: ArrayBuffer)` — accepts packed read requests
  - Binary format: `[count: u32] [rootKey, path, offset, length, requestId]...`
  - Dispatches each read to `Dispatchers.IO` (same `ioScope` as verified writes)
  - On completion: queue result in new `pendingDiskReadResults: ConcurrentLinkedQueue`
  - Results packed with `[requestId, resultCode, data]` and flushed at tick start alongside write results
- Extend `__jstorrent_file_flush()` (or add `__jstorrent_file_flush_reads()`) to drain read results too
- JS callback: `__jstorrent_file_dispatch_read_batch(packed)` delivers results

**TypeScript** (`packages/engine/src/adapters/native/`):
- New `native-async-read.ts` (or extend `native-batching-disk-queue.ts`):
  - `queueAsyncRead(rootKey, path, offset, length): Promise<Uint8Array>` — queues read, returns promise
  - Collect reads during tick, pack into single FFI call at UPLOAD phase
  - When results arrive via flush callback, resolve the corresponding promises
- Update `NativeFileHandle.read()` to use async path instead of sync `__jstorrent_file_read`

### 7. Tests — `packages/engine/src/core/torrent-uploader.test.ts`

- Per-peer request queuing and limit
- Watermark gating: reads stop when `sendBufferBytes + readingBytes >= watermark`
- Requests preserved when watermark full (not discarded)
- Rate limit respected independently
- Peer disconnect cleanup

---

## Detailed Flow

### Request Arrival (tick Phase 1 GATHER)
```
peer.drainBuffer() → parses REQUEST → emits 'request' event
  → uploader.queueRequest(peer, index, begin, length)
    → validate, add to peerState.requests[]
    → return (no reads triggered)
```

### Upload Serving (tick Phase 3.5 UPLOAD)
```
uploader.fillSendBuffers(connectedPeers):
  for each peer:
    if choked: discard queued requests, skip
    while requests queued:
      if sendBufferBytes + readingBytes >= watermark: break
      if !uploadBucket.tryConsume(length): return (rate limited)
      dequeue request, readingBytes += length
      contentStorage.read(index, begin, length).then(block => {
        readingBytes -= length
        peer.sendPiece(index, begin, block)  // appends to sendQueue
        recordUpload(block.length)
      })
```

### Async Read Lifecycle
```
Tick N:
  UPLOAD phase: issues reads → readingBytes += length, promises in-flight
  OUTPUT phase: flushes sendQueue (data from PRIOR completed reads only)

Between ticks (or same tick if fast):
  Async reads complete → readingBytes -= length
  peer.sendPiece() → sendQueue grows (no socket write yet)

Tick N+1:
  UPLOAD phase: sees updated sendQueueBytes, watermark gates new reads
  OUTPUT phase: flushes everything to sockets
```

Reads may complete same tick or take multiple ticks. The watermark adapts naturally.

---

## What This Fixes

1. **No JS thread blocking on Android**: Reads are async via Kotlin I/O threads
2. **No unnecessary choking**: Requests stay queued, served when buffer has room
3. **No 10-second recovery stalls**: No choke/unchoke cycle for backpressure
4. **Per-peer fairness**: Each peer's watermark is independent
5. **Android OOM protection**: Conservative 64KB watermark limits buffered data
6. **Unified code path**: Same TorrentUploader for all backends, different watermark configs

## Future Enhancements (not in this PR)

- Dynamic watermark based on upload speed (libtorrent's `send_buffer_watermark_factor`)
- Read cache (serve same block to multiple peers without re-reading)
- SUGGEST_PIECE for cached pieces

---

## Implementation Phases

Each phase produces a working, testable state. Run `pnpm run typecheck && pnpm run test && pnpm run lint && pnpm format:fix` after each.

### Phase 1: Pull-based uploader with per-peer queues (TypeScript only)

**Scope**: Refactor TorrentUploader to pull model. No Kotlin changes yet — Android still uses sync reads, but gated by watermark.

**Files**:
- `packages/engine/src/core/peer-connection.ts` — Add `get sendBufferBytes()` getter
- `packages/engine/src/core/torrent-uploader.ts` — Major refactor:
  - Per-peer state map, `fillSendBuffers()`, remove `drainQueue()`/`MAX_PENDING_READS`/`chokePeer`
  - Watermark check: `sendBufferBytes + readingBytes < SEND_BUFFER_WATERMARK`
  - Issue `contentStorage.read()` as promise, handle completion
- `packages/engine/src/core/torrent-tick-loop.ts` — Add UPLOAD phase (3.5)
- `packages/engine/src/core/torrent.ts` — Wire `fillSendBuffers` callback, remove `chokePeer`
- `packages/engine/src/presets/native.ts` — Remove `MAX_PENDING_READS = 8`, add `SEND_BUFFER_WATERMARK = 64 * 1024`
- `packages/engine/src/core/torrent-uploader.test.ts` — New/updated tests

**Verify**: Unit tests pass. Daemon seeding works (manual test). Android seeding works but reads still block JS thread (acceptable temporarily since watermark limits volume).

**Behavioral change**: Uploads are no longer push-based. No more backpressure choking. Requests preserved in queue instead of discarded. Per-peer fairness via independent watermarks.

### Phase 2: Async read batch on Android (Kotlin + TypeScript)

**Scope**: Make Android disk reads non-blocking. Same pattern as verified write batch.

**Files**:
- `android/.../FileBindings.kt` — Add:
  - `__jstorrent_file_read_batch(packed)` async FFI
  - `pendingDiskReadResults: ConcurrentLinkedQueue`
  - Extend flush to drain read results
  - JS dispatch callback `__jstorrent_file_dispatch_read_batch`
- `packages/engine/src/adapters/native/native-async-read.ts` (new) — Queue reads, pack for FFI, resolve promises on flush
- `packages/engine/src/adapters/native/native-file-handle.ts` — Use async read path
- `packages/engine/src/adapters/native/bindings.d.ts` — Declare new FFI functions

**Verify**: `./gradlew :app:compileDebugKotlin`. Android seeding with no JS thread blocking. Check tick timing logs — UPLOAD phase should be near-zero ms (just issuing reads, not waiting).

### Phase 3: Tuning and observability

**Scope**: Add metrics, tune watermarks, verify end-to-end.

**Files**:
- `packages/engine/src/core/torrent-uploader.ts` — Add stats: reads issued/completed per tick, watermark hits, queue depths
- `packages/engine/src/core/torrent-tick-loop.ts` — Log UPLOAD phase timing in 5-second tick stats

**Verify**:
- Android: Seed 1GB torrent, watch for OOM, stalls, upload speed
- Daemon: Benchmark upload throughput (`./scripts/benchmark-daemon-download.sh` from another client)
- Adjust watermarks based on observed behavior
