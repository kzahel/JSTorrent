# HTTP Batched Writes for ChromeOS

## Problem Statement

On ChromeOS, the extension communicates with the Android companion app via HTTP. Each piece write (typically 1MB) requires a separate HTTP request with ~7ms overhead. With downloads running at 60 MB/s but writes limited to ~20-30 MB/s, the disk queue fills up and backpressure throttles downloads.

## Design Goal

Batch multiple piece writes into a single HTTP request to reduce per-request overhead. Only batch when the disk queue is backed up (has pending writes) - don't add latency when writes can keep up.

## Current Implementation (BROKEN)

**Status:** The current implementation is incorrect and performs worse than no batching.

**Problem:** It replaces the disk queue entirely with a single-in-flight batching queue, which serializes all writes and kills the 5-worker parallelism.

### Files (to be rewritten)

| File | Purpose |
|------|---------|
| `packages/engine/src/adapters/daemon/http-batching-disk-queue.ts` | Batching queue (wrong approach - replaces disk queue) |
| `packages/engine/src/adapters/daemon/daemon-file-handle.ts` | Routes writes to batch queue |
| `packages/engine/src/presets/daemon.ts` | Wires batching for Node.js daemon client |
| `packages/client/src/engine-manager/chrome-extension-engine-manager.ts` | Wires batching for ChromeOS extension |

### Toggle Locations

**ChromeOS Extension** (`chrome-extension-engine-manager.ts:35`):
```typescript
const USE_BATCHED_WRITES = true  // Set to false to disable
```

**Node.js Daemon CLI** (`run-daemon-rpc.ts`):
```bash
# Environment variable
USE_BATCHED_WRITES=1 ./scripts/benchmark-daemon-download.sh

# Or CLI flag
--batched-writes
```

## Broken Batching Logic (Current)

The current implementation uses single in-flight, which serializes everything:

1. Write comes in
2. If no batch in-flight → send immediately
3. If batch in-flight → queue to buffer
4. When in-flight completes → flush queued batch
5. Result: Only 1 HTTP request at a time, ~6.5 MB/s throughput

**Why it's slow:** Single in-flight serializes all writes. We don't accumulate enough writes during the ~190ms HTTP window to reach meaningful batch sizes. Non-batched 5-worker parallel achieves 31 MB/s by overlapping HTTP latency.

## Benchmark

### Running the Benchmark

```bash
# Prerequisites:
# - Android companion app running on ChromeOS
# - 1GB test seeder: pnpm seed-for-test --size 1gb
# - Config in ~/.jstorrent-devices:
#     seeder=<ip>:6881
#     benchmark_host=chromebook

# Without batching
./scripts/benchmark-daemon-download.sh

# With batching
USE_BATCHED_WRITES=1 ./scripts/benchmark-daemon-download.sh
```

### Current Results (2025-01-30)

| Mode | Speed | Time | Notes |
|------|-------|------|-------|
| No batching | 31.0 MB/s | 33s | 5 parallel HTTP workers |
| With batching | 25.6 MB/s | 40s | Single in-flight, 2-3 writes/batch |

**Batching is currently slower.**

## Correct Design: Adaptive Batching with Worker Parallelism

The correct design keeps the 5-worker parallelism and adds adaptive batching at the worker level.

### Key Principles

1. **Keep TorrentDiskQueue with 5 workers** - Don't replace the queue, augment it
2. **Batch at worker execution time** - When a worker grabs a job, it can peek at the queue and grab additional pending jobs to batch
3. **Piece-size-aware** - Small pieces benefit more from batching (higher overhead ratio)
4. **Backlog-driven** - Only batch when there's actually backlog; don't add artificial delays
5. **Configurable thresholds** - Tune via benchmarks, not guesswork

### Adaptive Batching Logic

```
when worker grabs a job:
  piece_size = job.data.length
  backlog = queue.pending.length

  if piece_size >= SMALL_PIECE_THRESHOLD and backlog == 0:
    # Large piece, no backlog - send immediately (single piece)
    send single

  else if piece_size < SMALL_PIECE_THRESHOLD:
    # Small piece - always try to batch
    # Grab more from queue up to minBatchSizeForSmallPieces or maxBatchSize
    batch = [job] + grab_pending_up_to(minBatchSizeForSmallPieces)
    send batch

  else:
    # Large piece with backlog - batch opportunistically
    batch = [job] + grab_all_pending_up_to(maxBatchSize)
    send batch
```

### Why Piece Size Matters

HTTP overhead is ~7ms per request. The overhead ratio depends on piece size:

| Piece Size | Overhead Ratio | Batching Benefit |
|------------|----------------|------------------|
| 16 KB | 7ms / ~0.5ms transfer = 1400% | Critical |
| 256 KB | 7ms / ~8ms transfer = 87% | High |
| 1 MB | 7ms / ~33ms transfer = 21% | Moderate |
| 4 MB | 7ms / ~133ms transfer = 5% | Low |

Small pieces have catastrophic overhead ratios - batching them is essential. Large pieces can be sent individually without much penalty.

### Configuration

```typescript
interface AdaptiveBatchingConfig {
  /** Below this size, batch aggressively even with minimal backlog */
  smallPieceThreshold: number  // Default: 1MB (tune via benchmark)

  /** For small pieces, try to accumulate at least this much before sending */
  minBatchSizeForSmallPieces: number  // Default: 2MB (tune via benchmark)

  /** Never exceed this batch size */
  maxBatchSize: number  // Default: 16MB

  /** Optional cap on pieces per batch */
  maxPiecesPerBatch?: number
}
```

All thresholds should be tunable via environment variables or CLI flags for benchmarking:

```bash
SMALL_PIECE_THRESHOLD=512KB MIN_BATCH_SIZE=4MB ./scripts/benchmark-daemon-download.sh
```

### Implementation Location

The batching logic belongs in the worker's execute callback, NOT in a replacement disk queue:

```
TorrentDiskQueue.execute() callback:
  → Check backlog and piece size
  → Optionally grab more jobs from queue
  → DaemonFileHandle.write() with batched data
  → Or: Dedicated batch endpoint that accepts multiple pieces
```

This preserves the 5-worker parallelism while allowing each worker to independently batch when beneficial.

## Architecture Notes

### Target Disk Queue Flow (Correct Design)

```
TorrentContentStorage.writePiece()
  → diskQueue.enqueue(job, execute callback)
    → Worker picks up job
    → Worker peeks queue for additional jobs (based on piece size + backlog)
    → Worker batches jobs if beneficial
    → DaemonFileHandle.writeBatch() or write()
      → HTTP POST /write-batch/{rootKey} (batched)
      → Or: direct HTTP/WebSocket write (single)
```

The disk queue (5 workers) controls concurrency. Each worker independently decides whether to batch based on piece size and queue backlog.

### Why Batching Needs WebSocket

Batch writes return HTTP 202 Accepted immediately. Results (success/hash mismatch) come back via WebSocket ACK frames. This allows the HTTP connection to be reused while waiting for disk I/O on the companion.

## Related Files

- `docs/performance/batched-http-writes-plan.md` - Original design plan
- `android/companion-server/src/main/java/.../BatchWriteResults.kt` - Android batch write handling
- `packages/engine/test/adapters/daemon/http-batching-disk-queue.test.ts` - Unit tests
