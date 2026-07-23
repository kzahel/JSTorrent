# Adaptive HTTP Batching Implementation Plan

## Problem

Current `HttpBatchingDiskQueue` replaces the disk queue entirely with single-in-flight batching, serializing all writes and killing 5-worker parallelism (6.5 MB/s vs 31 MB/s unbatched).

## Solution

Worker-driven batching: when a worker executes a job, it checks queue depth and grabs additional pending jobs to batch together.

- Keep `TorrentDiskQueue` with 5 workers unchanged
- Add `grabPending()` method for workers to atomically grab extra jobs
- Worker decides batch size based on `pendingBytes`
- Large backlog = grab more jobs = bigger batch
- Small backlog = single write for low latency

## Architecture

```
TorrentContentStorage.writePiece()
  → diskQueue.enqueue(job, execute callback)
    → Worker executes
      → Check diskQueue.pendingBytes
      → If backlog: grabPending() for extra jobs
      → DaemonFileHandle.writeBatch([job, ...extras])
      → Single HTTP request for entire batch
      → All jobs complete together
```

## Batching Logic (in execute callback)

```typescript
diskQueue.enqueue(job, async (job) => {
  const pendingBytes = diskQueue.pendingBytes

  if (pendingBytes < lowThreshold) {
    // Queue small - send single for low latency
    await fileHandle.write(job.file, job.offset, job.data)
  } else {
    // Backlog exists - batch aggressively
    const extras = diskQueue.grabPending(maxBatchBytes, maxBatchCount)
    const allJobs = [job, ...extras]
    await fileHandle.writeBatch(allJobs)
  }
})
```

Key insight: The queue depth naturally reflects whether writes are keeping up with downloads. No complex throughput tracking or accumulation state.

## Configuration

```typescript
interface BatchingConfig {
  /** Queue depth below which we send singles. Default: 5MB */
  lowBacklogThreshold?: number

  /** Max bytes to grab for a batch. Default: 16MB */
  maxBatchBytes?: number

  /** Max pieces per batch. Default: 64 */
  maxBatchCount?: number
}
```

Configurable via env vars for benchmarking:
```bash
LOW_BACKLOG_MB=5 MAX_BATCH_MB=16 ./scripts/benchmark-daemon-download.sh
```

## Changes to TorrentDiskQueue

Add direct accessors and grab method:

```typescript
interface IDiskQueue {
  // ... existing methods ...

  /** Total bytes in pending jobs */
  readonly pendingBytes: number

  /** Number of jobs waiting for a worker */
  readonly pendingCount: number

  /** Atomically dequeue pending jobs up to limits for batching */
  grabPending(maxBytes: number, maxCount: number): DiskJob[]
}
```

Implementation:

```typescript
class TorrentDiskQueue {
  private _pendingBytes = 0  // Track incrementally

  get pendingBytes(): number {
    return this._pendingBytes
  }

  get pendingCount(): number {
    return this.pending.length
  }

  // Called on enqueue
  private addPending(job: DiskJob) {
    this.pending.push(job)
    this._pendingBytes += job.data.length
  }

  // Called when worker starts job (existing logic)
  private removePending(job: DiskJob) {
    // ... existing removal ...
    this._pendingBytes -= job.data.length
  }

  grabPending(maxBytes: number, maxCount: number): DiskJob[] {
    const grabbed: DiskJob[] = []
    let grabbedBytes = 0

    while (
      this.pending.length > 0 &&
      grabbed.length < maxCount &&
      grabbedBytes < maxBytes
    ) {
      const job = this.pending.shift()!
      this._pendingBytes -= job.data.length
      grabbedBytes += job.data.length
      grabbed.push(job)
    }

    return grabbed
  }
}
```

## Changes to DaemonFileHandle

Add `writeBatch()` method:

```typescript
class DaemonFileHandle {
  // Existing single write
  async write(offset: number, data: Uint8Array): Promise<void>

  // New batch write
  async writeBatch(writes: Array<{offset: number, data: Uint8Array}>): Promise<void> {
    const packed = packVerifiedWriteBatch(this.rootKey, writes)
    // Send single HTTP request with all writes
    // Results via WebSocket ACK (already implemented)
  }
}
```

Reuse `packVerifiedWriteBatch()` from existing implementation.

## Files to Modify

| File | Action |
|------|--------|
| `packages/engine/src/core/disk-queue.ts` | Add `pendingBytes`, `pendingCount`, `grabPending()` |
| `packages/engine/src/adapters/daemon/daemon-file-handle.ts` | Add `writeBatch()` method |
| `packages/engine/src/core/torrent-content-storage.ts` | Batching logic in execute callback |
| `packages/engine/src/adapters/daemon/http-batching-disk-queue.ts` | DELETE (move `packVerifiedWriteBatch` to utils first) |

## Implementation Phases

### Phase 1: Disk Queue Infrastructure

**Goal**: Add queue depth visibility and grab capability without changing behavior.

**Changes**:
- `disk-queue.ts`: Add `_pendingBytes` tracking (increment on enqueue, decrement on start/grab)
- `disk-queue.ts`: Add `pendingBytes`, `pendingCount` getters
- `disk-queue.ts`: Add `grabPending(maxBytes, maxCount)` method
- Update `IDiskQueue` interface

**Verification**:
```bash
pnpm run typecheck
pnpm run test -- disk-queue  # Existing tests still pass
pnpm run lint
```

Add unit tests for new methods:
- `pendingBytes` increments/decrements correctly
- `grabPending()` returns correct jobs and updates `pendingBytes`
- `grabPending()` respects maxBytes and maxCount limits

---

### Phase 2: Batch Write Capability

**Goal**: Add `writeBatch()` to DaemonFileHandle without changing existing write path.

**Changes**:
- Move `packVerifiedWriteBatch()` from `http-batching-disk-queue.ts` to `daemon-file-handle.ts` (or a utils file)
- `daemon-file-handle.ts`: Add `writeBatch()` method
- Single HTTP POST with packed batch, results via WebSocket ACK

**Verification**:
```bash
pnpm run typecheck
pnpm run test
pnpm run lint
```

Add unit test for `writeBatch()`:
- Correctly packs multiple writes
- Sends single HTTP request
- Resolves when all ACKs received

---

### Phase 3: Wire Up Batching Logic

**Goal**: Enable adaptive batching in the execute callback.

**Changes**:
- `torrent-content-storage.ts`: Add batching logic in execute callback
- Check `diskQueue.pendingBytes`, call `grabPending()` when backlog exists
- Use `writeBatch()` for batched writes, `write()` for singles
- Add `USE_ADAPTIVE_BATCHING` env var toggle (default: off)

**Verification**:
```bash
pnpm run typecheck
pnpm run test
pnpm run lint

# Baseline (no batching) - record result
./scripts/benchmark-daemon-download.sh

# With batching enabled - should match or exceed baseline
USE_ADAPTIVE_BATCHING=1 ./scripts/benchmark-daemon-download.sh
```

**Success criteria**: Batched throughput >= unbatched throughput (31 MB/s baseline).

---

### Phase 4: Threshold Tuning

**Goal**: Find optimal threshold values via benchmarking.

**Test matrix**:
```bash
# Vary lowBacklogThreshold
for low in 1 2 5 10; do
  LOW_BACKLOG_MB=$low USE_ADAPTIVE_BATCHING=1 ./scripts/benchmark-daemon-download.sh
done

# Vary maxBatchBytes
for max in 4 8 16 32; do
  MAX_BATCH_MB=$max USE_ADAPTIVE_BATCHING=1 ./scripts/benchmark-daemon-download.sh
done

# Vary maxBatchCount
for count in 16 32 64 128; do
  MAX_BATCH_COUNT=$count USE_ADAPTIVE_BATCHING=1 ./scripts/benchmark-daemon-download.sh
done
```

**Record results** in a table and pick best defaults.

**Verification**: Update default values in code, re-run benchmark to confirm.

---

### Phase 5: Cleanup

**Goal**: Remove old broken implementation.

**Changes**:
- Delete `http-batching-disk-queue.ts`
- Remove old `USE_BATCHED_WRITES` toggle and related wiring
- Update imports in any affected files
- Enable batching by default (or keep toggle for A/B testing)

**Verification**:
```bash
pnpm run typecheck
pnpm run test
pnpm run lint

# Final benchmark with defaults
./scripts/benchmark-daemon-download.sh
```

---

## Future Optimizations (not in initial implementation)

- **Consecutive piece ordering**: When grabbing, prefer pieces that are consecutive in the file for better disk locality
- **Per-file batching**: Only batch writes to the same file
- **Adaptive thresholds**: Adjust thresholds based on observed throughput
