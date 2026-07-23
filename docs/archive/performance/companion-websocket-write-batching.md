# Companion WebSocket Write Batching

## Problem Statement

Android companion mode WebSocket writes achieve significantly lower throughput than Android standalone mode, despite using the same underlying I/O infrastructure (SAF file writes, SHA1 hashing).

**Observed Performance:**
| Mode | Write Throughput | Notes |
|------|------------------|-------|
| Standalone Android | ~60 MB/s | Batched FFI, parallel execution |
| Companion WebSocket | ~7 MB/s | Per-write ACK round-trip |
| Companion HTTP POST | ~15 MB/s | Slightly better, still per-write |

The ~8x throughput gap between standalone and companion WebSocket writes cannot be explained by SAF overhead alone, since both modes use SAF for actual disk I/O.

## Architecture Comparison

### Standalone Android (Fast Path)

```
Tick N:
  Piece 1 completes → queueVerifiedWrite() → adds to local queue, returns Promise
  Piece 2 completes → queueVerifiedWrite() → adds to local queue, returns Promise
  Piece 3 completes → queueVerifiedWrite() → adds to local queue, returns Promise
  ...
  End of tick → flushBatchedWrites():
    - Pack all N writes into single binary buffer
    - Single FFI call: __jstorrent_file_write_verified_batch(packed)
    - Kotlin unpacks and launches N parallel coroutines
    - Each coroutine: hash → SAF write → queue result
    - Results accumulate in ConcurrentLinkedQueue

Tick N+1:
  Start of tick → __jstorrent_file_flush():
    - Drain all results from queue
    - Pack into single binary buffer
    - Single FFI return with all results
    - JS unpacks and resolves all Promises
```

**Key characteristics:**
- Writes are batched at tick boundary (1 FFI call for N writes)
- Results are batched at tick boundary (1 FFI return for N results)
- Kotlin coroutines run truly in parallel on Dispatchers.IO
- No round-trip latency per write

### Companion WebSocket (Slow Path)

```
Piece 1 completes → handle.write():
  - Build WS frame with requestId
  - Send frame to companion
  - Register Promise in pendingWrites map
  - Await ACK...
                                          Companion receives frame
                                          → scope.launch(Dispatchers.IO)
                                          → hash data
                                          → SAF open/write/close
                                          → send ACK frame with requestId
  ← ACK arrives
  - Resolve Promise
  - Write 1 complete

Piece 2 completes → handle.write():
  ... same round-trip ...
```

**Key characteristics:**
- Each write is a full WebSocket round-trip
- Even with 6 disk queue workers, each awaits its own ACK
- Latency compounds: 6 writes × 100ms round-trip = 600ms for 6MB = 10 MB/s
- No batching of writes or results

## Root Cause Analysis

The fundamental issue is **per-write synchronous ACK waiting**:

1. **Extension side**: `DaemonFileHandle.writeViaWebSocket()` creates a Promise that only resolves when the ACK frame arrives:
   ```typescript
   return new Promise((resolve, reject) => {
     pendingWrites.set(requestId, { resolve, reject })
     this.connection.sendFrame(frame)
     // Promise hangs until ACK arrives...
   })
   ```

2. **Companion side**: `IoWebSocketHandler.handleFileWrite()` processes writes correctly in parallel via coroutines, but sends individual ACK per write:
   ```kotlin
   scope.launch(Dispatchers.IO) {
     // hash + write
     sendFileWriteAck(requestId, rootKey, offset)  // Individual ACK
   }
   ```

3. **Round-trip latency**: Even on localhost, WebSocket frame send → receive → process → ACK → receive adds latency. With SAF write time (~50-100ms on battery), each write takes 100-200ms round-trip.

4. **Disk queue doesn't help**: The 6-worker disk queue allows 6 concurrent writes, but each worker blocks awaiting its ACK. This gives 6× parallelism but doesn't eliminate the round-trip overhead.

## Recommended Solution

Implement the same batching pattern used by standalone Android:

### Phase 1: Extension-Side Write Batching

Create `DaemonBatchingDiskQueue` (similar to `NativeBatchingDiskQueue`):

```typescript
class DaemonBatchingDiskQueue implements IDiskQueue {
  private pending: PendingVerifiedWrite[] = []

  queueVerifiedWrite(
    rootKey: string,
    path: string,
    position: number,
    data: ArrayBuffer,
    expectedHash: Uint8Array,
  ): Promise<{ bytesWritten: number }> {
    return new Promise((resolve, reject) => {
      const callbackId = `vw_${nextCallbackId++}`
      // Register callback for batch result
      pendingWriteCallbacks.set(callbackId, { resolve, reject })
      // Queue locally - no network call yet
      this.pending.push({ rootKey, path, position, data, expectedHash, callbackId })
    })
  }

  flushPending(): void {
    if (this.pending.length === 0) return
    // Pack all writes into single binary buffer
    const packed = packVerifiedWriteBatch(this.pending)
    // Single WebSocket frame for all writes
    this.connection.sendFrame(buildFileWriteBatchFrame(packed))
    this.pending = []
  }
}
```

### Phase 2: New WebSocket Opcode for Batch Writes

Add `OP_FILE_WRITE_BATCH = 0x33` to Protocol:

**Request frame format:**
```
[envelope:8][count:4 LE] then for each write:
  [rootKeyLen:1][rootKey:N]
  [pathLen:2 LE][path:M]
  [offset:8 LE]
  [flags:1][optional sha1:20]
  [dataLen:4 LE][data:K]
  [callbackIdLen:1][callbackId:J]
```

**Response frame format (batched ACKs):**
```
[envelope:8][count:4 LE] then for each result:
  [callbackIdLen:1][callbackId:N]
  [resultCode:1]  // 0=success, 1=hash_mismatch, 2=io_error
  [bytesWritten:4 LE]  // -1 on error
```

### Phase 3: Companion-Side Batch Handler

```kotlin
private fun handleFileWriteBatch(requestId: Int, payload: ByteArray) {
    val writes = unpackWriteBatch(payload)
    val results = ConcurrentLinkedQueue<WriteResult>()
    val latch = CountDownLatch(writes.size)

    // Launch all writes in parallel
    for (write in writes) {
        scope.launch(Dispatchers.IO) {
            try {
                // Hash verification
                if (write.expectedHash != null) {
                    val actualHash = Hasher.sha1(write.data)
                    if (!actualHash.contentEquals(write.expectedHash)) {
                        results.add(WriteResult(write.callbackId, HASH_MISMATCH, -1))
                        latch.countDown()
                        return@launch
                    }
                }
                // SAF write
                fileManager.write(write.rootUri, write.path, write.offset, write.data)
                results.add(WriteResult(write.callbackId, SUCCESS, write.data.size))
            } catch (e: Exception) {
                results.add(WriteResult(write.callbackId, IO_ERROR, -1))
            } finally {
                latch.countDown()
            }
        }
    }

    // Wait for all writes to complete, then send single batch response
    scope.launch {
        latch.await()
        sendFileWriteBatchAck(requestId, results.toList())
    }
}
```

### Phase 4: Engine Integration

In `BtEngine.tick()`, flush batched writes at end of tick:

```typescript
private tick(): void {
  // ... existing tick logic ...

  // Flush batched writes (works for both native and daemon modes)
  if (this.batchingDiskQueue) {
    this.batchingDiskQueue.flushPending()
  }
}
```

## Implementation Plan

1. **Add Protocol opcodes** (`Protocol.kt`)
   - `OP_FILE_WRITE_BATCH = 0x33`
   - `OP_FILE_WRITE_BATCH_ACK = 0x34`

2. **Create DaemonBatchingDiskQueue** (`packages/engine/src/adapters/daemon/`)
   - Mirror `NativeBatchingDiskQueue` API
   - Pack/unpack binary format matching Kotlin side

3. **Add batch handler to IoWebSocketHandler** (`IoWebSocketHandler.kt`)
   - `handleFileWriteBatch()` - unpack, parallel execute, batch response
   - Reuse existing hash/write logic

4. **Wire up in daemon preset** (`packages/engine/src/presets/daemon.ts`)
   - Create `DaemonBatchingDiskQueue` when using WebSocket connection
   - Pass to engine for tick-based flushing

5. **Add flushPending call to engine tick** (`bt-engine.ts`)
   - Call at end of tick, before scheduling next tick

## Expected Results

| Metric | Before | After |
|--------|--------|-------|
| Writes per WS frame | 1 | N (batch size) |
| Round-trips per tick | N | 1 |
| Throughput | ~7 MB/s | ~40-50 MB/s (estimated) |

The remaining gap to standalone (~60 MB/s) would be due to:
- WebSocket frame overhead vs FFI
- Chrome ↔ extension message passing
- Additional memory copies in WS path

## Alternative Considered: Fire-and-Forget Writes

A simpler approach would be fire-and-forget writes (don't wait for ACK):
- Pro: Minimal code changes
- Con: No error handling, no hash mismatch detection
- Con: Can't implement backpressure

Rejected because hash mismatch handling is critical for data integrity.

## Files to Modify

**New files:**
- `packages/engine/src/adapters/daemon/daemon-batching-disk-queue.ts`

**Modified files:**
- `android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt` - new opcodes
- `android/companion-server/.../IoWebSocketHandler.kt` - batch handler
- `packages/engine/src/presets/daemon.ts` - wire up batching queue
- `packages/engine/src/core/bt-engine.ts` - flush at tick boundary
- `packages/engine/src/adapters/daemon/daemon-file-handle.ts` - use batching queue

## References

- `packages/engine/src/adapters/native/native-batching-disk-queue.ts` - reference implementation
- `android/quickjs-engine/.../FileBindings.kt` - Kotlin batch handling reference
- `docs/chromeos-companion-throughput.md` - prior investigation
