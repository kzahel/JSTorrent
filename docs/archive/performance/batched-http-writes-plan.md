# Batched HTTP Writes Implementation Plan

**Goal:** Improve daemon write throughput from ~29 MB/s to 100+ MB/s by batching multiple pieces into single HTTP requests and returning results via WebSocket.

**Current bottleneck:** ~7ms HTTP overhead per request × 5 concurrent writes = throughput ceiling

**Target:** 16-32MB batches (16-32 pieces at 1MB each), results streamed via WebSocket

## Architecture

```
JS Engine                          Kotlin Companion
─────────                          ────────────────

[Piece completes] ──┐
[Piece completes] ──┼─► BatchBuffer (16-32MB)
[Piece completes] ──┘
        │
        ▼ (threshold or timeout)
[Pack binary] ─────► HTTP POST /write-batch/{rootKey}
        │                                  │
        │                                  ▼
        │                          [Unpack batch]
        │                          [Launch parallel writes]
        │                                  │
        │              ┌───────────────────┼───────────────────┐
        │              ▼                   ▼                   ▼
        │         [Write 1]           [Write 2]           [Write N]
        │              │                   │                   │
        │              ▼                   ▼                   ▼
        │         [Hash+Disk]         [Hash+Disk]         [Hash+Disk]
        │              │                   │                   │
        │              └───────────────────┴───────────────────┘
        │                                  │
        ▼                                  ▼
[pendingWrites Map] ◄────── WebSocket ACK/ERROR frames (per piece)
        │
        ▼
[Resolve/reject promises]
```

## Binary Protocol

Reuse existing native batch format from `FileBindings.kt`:

**Request (HTTP POST body):**
```
[count: u32 LE]
for each write:
  [rootKeyLen: u8] [rootKey: UTF-8]
  [pathLen: u16 LE] [path: UTF-8]
  [position: u64 LE]
  [dataLen: u32 LE] [data: bytes]
  [hashHex: 40 bytes]
  [callbackIdLen: u8] [callbackId: UTF-8]
```

**Response:** HTTP 202 Accepted (empty body, results via WebSocket)

**WebSocket results:** Existing `OP_FILE_WRITE_ACK` (0x31) / `OP_FILE_WRITE_ERROR` (0x32) frames

---

## Phase 1: Kotlin Endpoint (Server-Side)

### Architecture Decision: Shared Queue + Broadcast

**Simplification:** This is a single-user scenario. Typically one client connected at a time.
- No need to route results to specific clients
- Broadcast ACK/ERROR frames to all connected WebSocket clients
- JS client matches results by `callbackId` (already in `pendingWrites` map)

**Flow:**
```
HTTP POST /write-batch
        │
        ▼
[Unpack batch, launch coroutines]
        │
        ├─► Write 1 ─► complete ─► BatchWriteResults.pending.add(result)
        ├─► Write 2 ─► complete ─► BatchWriteResults.pending.add(result)
        └─► Write N ─► complete ─► BatchWriteResults.pending.add(result)
                                            │
                                            ▼
                                   wsServer.notifyResults()
                                            │
                                            ▼
                                   [Drain queue, broadcast to all WS clients]
```

### 1.1 Create shared result queue

**File:** `android/companion-server/src/main/java/com/jstorrent/companion/server/BatchWriteResults.kt` (new)

```kotlin
package com.jstorrent.companion.server

import java.util.concurrent.ConcurrentLinkedQueue

/**
 * Write result for batch processing.
 * Matches existing OP_FILE_WRITE_ACK/ERROR frame format.
 */
data class WriteResult(
    val callbackId: String,
    val bytesWritten: Int,
    val resultCode: Int  // 0=SUCCESS, 1=HASH_MISMATCH, 2=IO_ERROR, 3=INVALID_ARGS
)

/**
 * Shared queue for batch write results.
 * HTTP handler adds results, WebSocket server drains and broadcasts.
 */
object BatchWriteResults {
    val pending = ConcurrentLinkedQueue<WriteResult>()

    @Volatile
    private var notifyCallback: (() -> Unit)? = null

    /**
     * Register callback to be invoked when results are available.
     * Called by JavaWebSocketServer during initialization.
     */
    fun setNotifyCallback(callback: () -> Unit) {
        notifyCallback = callback
    }

    /**
     * Add a result and notify the WebSocket server.
     * Called by write coroutines when each write completes.
     */
    fun addResult(callbackId: String, bytesWritten: Int, resultCode: Int) {
        pending.add(WriteResult(callbackId, bytesWritten, resultCode))
        notifyCallback?.invoke()
    }

    /**
     * Drain all pending results.
     * Called by WebSocket server to get results for broadcasting.
     */
    fun drain(): List<WriteResult> {
        val results = mutableListOf<WriteResult>()
        while (true) {
            val result = pending.poll() ?: break
            results.add(result)
        }
        return results
    }
}
```

### 1.2 Add `/write-batch/{rootKey}` endpoint to NettyHttpServer

**File:** `android/companion-server/src/main/java/com/jstorrent/companion/server/NettyHttpServer.kt`

**Changes:**

1. Add route in `channelRead0()`:
```kotlin
path.startsWith("/write-batch/") && method == HttpMethod.POST -> handleWriteBatch(ctx, request, path)
```

2. Add handler method:
```kotlin
private fun handleWriteBatch(ctx: ChannelHandlerContext, request: FullHttpRequest, path: String) {
    // Auth check (same as handleWrite)
    if (getExtensionHeaders(request) == null && !isStandaloneAuth(request)) {
        sendError(ctx, request, HttpResponseStatus.BAD_REQUEST, "Missing extension headers")
        return
    }
    if (!validateAuth(request)) {
        sendError(ctx, request, HttpResponseStatus.UNAUTHORIZED, "Invalid token")
        return
    }

    val rootKey = path.removePrefix("/write-batch/")
    if (rootKey.isBlank()) {
        sendError(ctx, request, HttpResponseStatus.BAD_REQUEST, "Missing root_key")
        return
    }

    val rootUri = deps.rootStore.resolveKey(rootKey)
    if (rootUri == null) {
        sendError(ctx, request, HttpResponseStatus.FORBIDDEN, "Invalid root key")
        return
    }

    // Read packed batch from body
    val content = request.content()
    val packed = ByteArray(content.readableBytes())
    content.readBytes(packed)

    // Unpack batch (reuse from FileBindings)
    val writes = try {
        unpackVerifiedWriteBatch(packed)
    } catch (e: Exception) {
        Log.e(TAG, "Failed to unpack batch: ${e.message}")
        sendError(ctx, request, HttpResponseStatus.BAD_REQUEST, "Invalid batch format")
        return
    }

    Log.i(TAG, "WRITE-BATCH: ${writes.size} writes for root $rootKey")

    // Launch all writes in parallel on IO dispatcher
    val scope = CoroutineScope(Dispatchers.IO)
    for (write in writes) {
        scope.launch {
            try {
                // Hash verification
                val actualHash = Hasher.sha1Hex(write.data)
                if (!actualHash.equals(write.expectedHashHex, ignoreCase = true)) {
                    BatchWriteResults.addResult(write.callbackId, -1, WriteResultCode.HASH_MISMATCH)
                    return@launch
                }

                // Write to disk
                fileManager.write(rootUri, write.path, write.position, write.data)
                BatchWriteResults.addResult(write.callbackId, write.data.size, WriteResultCode.SUCCESS)

            } catch (e: Exception) {
                Log.e(TAG, "Batch write failed: ${write.path}", e)
                BatchWriteResults.addResult(write.callbackId, -1, WriteResultCode.IO_ERROR)
            }
        }
    }

    // Return 202 Accepted immediately (results come via WebSocket)
    sendResponse(ctx, request, HttpResponseStatus.ACCEPTED, "text/plain", "Accepted ${writes.size} writes")
}
```

3. Import `unpackVerifiedWriteBatch` from FileBindings or copy the function to a shared location.

### 1.3 Wire up WebSocket broadcast

**File:** `android/companion-server/src/main/java/com/jstorrent/companion/server/JavaWebSocketServer.kt`

**Changes:**

1. Register notify callback during server start:
```kotlin
override fun onStart() {
    Log.i(TAG, "WebSocket server started on port $port")

    // Register to receive batch write results
    BatchWriteResults.setNotifyCallback {
        drainAndBroadcastResults()
    }
}
```

2. Add drain and broadcast method:
```kotlin
private fun drainAndBroadcastResults() {
    val results = BatchWriteResults.drain()
    if (results.isEmpty()) return

    for (result in results) {
        val frame = packAckOrErrorFrame(result)
        broadcast(frame)  // Send to all connected clients
    }
}

private fun packAckOrErrorFrame(result: WriteResult): ByteArray {
    // Pack as existing OP_FILE_WRITE_ACK (0x31) or OP_FILE_WRITE_ERROR (0x32) frame
    // Format: [version:1][opcode:1][flags:2][requestId:4][payload...]
    //
    // For ACK: payload is [status:1] (0 = success)
    // For ERROR: payload is [root_key_len:1][root_key][offset:8][error_code:4][message]
    //
    // Since we're using callbackId (string) not requestId (int), we need to encode differently.
    // Option A: Use a new opcode for batch results
    // Option B: Encode callbackId in payload
    //
    // Using Option B for compatibility with existing frame handler:
    // Payload: [callbackIdLen:1][callbackId:bytes][bytesWritten:4][resultCode:1]

    val opcode = if (result.resultCode == 0) 0x31 else 0x32  // ACK or ERROR
    val callbackIdBytes = result.callbackId.toByteArray(Charsets.UTF_8)

    val frameSize = 8 + 1 + callbackIdBytes.size + 4 + 1
    val buffer = ByteBuffer.allocate(frameSize).order(ByteOrder.LITTLE_ENDIAN)

    // Envelope
    buffer.put(1)  // version
    buffer.put(opcode.toByte())
    buffer.putShort(0)  // flags
    buffer.putInt(0)  // requestId (0 = use callbackId in payload)

    // Payload
    buffer.put(callbackIdBytes.size.toByte())
    buffer.put(callbackIdBytes)
    buffer.putInt(result.bytesWritten)
    buffer.put(result.resultCode.toByte())

    return buffer.array()
}
```

**Note:** The frame format above differs slightly from the existing single-write ACK format.
The JS frame handler will need to detect `requestId == 0` and parse callbackId from payload.
Alternatively, we could use a new opcode (e.g., `OP_FILE_WRITE_BATCH_ACK = 0x33`).

### 1.4 Update JS frame handler (minor)

**File:** `packages/engine/src/adapters/daemon/daemon-file-handle.ts`

**Changes:** Handle the batch ACK format (requestId == 0, callbackId in payload):

```typescript
connection.onFrame((frame) => {
    const view = new DataView(frame)
    const opcode = view.getUint8(1)
    const requestId = view.getUint32(4, true)

    if (opcode === OP_FILE_WRITE_ACK || opcode === OP_FILE_WRITE_ERROR) {
        if (requestId === 0) {
            // Batch result: parse callbackId from payload
            const callbackIdLen = view.getUint8(8)
            const callbackIdBytes = new Uint8Array(frame, 9, callbackIdLen)
            const callbackId = new TextDecoder().decode(callbackIdBytes)
            const bytesWritten = view.getInt32(9 + callbackIdLen, true)
            const resultCode = view.getUint8(9 + callbackIdLen + 4)

            const pending = pendingBatchWrites.get(callbackId)
            if (pending) {
                pendingBatchWrites.delete(callbackId)
                if (resultCode === 0) {
                    pending.resolve({ bytesWritten })
                } else {
                    pending.reject(new Error(`Write failed: code ${resultCode}`))
                }
            }
        } else {
            // Existing single-write ACK handling
            // ...
        }
    }
})
```

### 1.5 Tests

**Unit test for Kotlin:**
```kotlin
// BatchWriteResultsTest.kt
@Test
fun `addResult and drain works correctly`() {
    BatchWriteResults.addResult("cb1", 1024, 0)
    BatchWriteResults.addResult("cb2", -1, 1)

    val results = BatchWriteResults.drain()
    assertEquals(2, results.size)
    assertEquals("cb1", results[0].callbackId)
    assertEquals(0, results[0].resultCode)

    // Queue should be empty now
    assertTrue(BatchWriteResults.drain().isEmpty())
}
```

**Manual integration test:**

1. Start companion server
2. Connect WebSocket client (wscat or browser)
3. POST packed batch to `/write-batch/{rootKey}`:
```bash
# Create test batch (Python script or similar)
python3 -c "
import struct
# Pack 2 test writes... (see binary format above)
" | curl -X POST -H 'X-JST-Auth: <token>' \
    --data-binary @- \
    http://100.115.92.2:7800/write-batch/testroot
```
4. Observe ACK frames on WebSocket connection
5. Verify files written correctly

**Benchmark verification:**
- Not yet integrated with JS batching, so no throughput change expected
- Verify endpoint accepts requests and sends WebSocket responses correctly

---

## Phase 2: JS Batching Infrastructure

### 2.1 Create `HttpBatchingDiskQueue`

**File:** `packages/engine/src/adapters/daemon/http-batching-disk-queue.ts` (new)

**Responsibilities:**
- Collect verified writes into a batch buffer
- Track pending promises keyed by callbackId
- Flush when: size threshold (16MB default) OR timeout (100ms) OR explicit flush
- Pack batch using existing `packVerifiedWriteBatch()` format
- POST to `/write-batch/{rootKey}`

**Interface:**
```typescript
interface HttpBatchingDiskQueue {
  queueVerifiedWrite(
    rootKey: string,
    path: string,
    position: number,
    data: ArrayBuffer,
    expectedHash: Uint8Array
  ): Promise<{ bytesWritten: number }>

  flush(): Promise<void>  // Force flush current batch

  // Config
  readonly batchSizeThreshold: number  // bytes, default 16MB
  readonly batchTimeoutMs: number      // ms, default 100
}
```

### 2.2 Integrate with DaemonFileHandle

**File:** `packages/engine/src/adapters/daemon/daemon-file-handle.ts`

**Changes:**
- Add option to use batching queue for writes
- When batching enabled, route `writeVerified()` to `HttpBatchingDiskQueue`
- Results come via existing WebSocket frame handler (already wired up)

### 2.3 Wire up WebSocket result handling

**File:** `packages/engine/src/adapters/daemon/daemon-file-handle.ts`

**Changes:**
- Ensure frame handler processes ACK/ERROR for batch writes
- May need to unify callbackId format between single and batch writes

### 2.4 Tests

**Unit tests:**
```typescript
// http-batching-disk-queue.test.ts
- 'should pack writes in correct binary format'
- 'should flush when size threshold reached'
- 'should flush on timeout'
- 'should resolve promises on ACK'
- 'should reject promises on ERROR'
```

**Integration test (mock HTTP + WebSocket):**
- Verify full round-trip with mocked server

---

## Phase 3: Integration & Benchmark

### 3.1 Enable batching in daemon preset

**File:** `packages/engine/src/presets/daemon.ts`

**Changes:**
- Add config option: `useBatchedWrites: boolean`
- When enabled, create `HttpBatchingDiskQueue` and wire to file handles

### 3.2 Benchmark comparison

**Test script:** `scripts/benchmark-daemon-download.sh`

**Metrics to capture:**
- Average throughput (MB/s)
- Batch size distribution (pieces per batch)
- WebSocket ACK latency
- Disk queue depth over time

**Expected results:**
- Baseline: ~29 MB/s (current)
- Target: 80-150 MB/s (depends on disk)

### 3.3 Tests

**Benchmark verification:**
```bash
# Run with batching disabled (baseline)
./scripts/benchmark-daemon-download.sh
# Expected: ~29 MB/s

# Run with batching enabled
USE_BATCHED_WRITES=1 ./scripts/benchmark-daemon-download.sh
# Expected: significant improvement
```

---

## Phase 4: Optimization

### 4.1 Buffer pooling

**Goal:** Reduce GC pressure from allocating batch buffers

**Approach:**
- Pre-allocate 2-4 buffers of 32MB each
- Rotate buffers: fill one while previous is in-flight
- Return buffer to pool when all ACKs received

### 4.2 Tune batch parameters

**Parameters to tune:**
- `batchSizeThreshold`: 8MB, 16MB, 32MB
- `batchTimeoutMs`: 50ms, 100ms, 200ms
- `maxConcurrentBatches`: 1, 2, 4

**Test matrix:**
| Size | Timeout | Concurrent | Throughput |
|------|---------|------------|------------|
| 16MB | 100ms   | 2          | ???        |
| 32MB | 100ms   | 2          | ???        |
| 16MB | 50ms    | 4          | ???        |

### 4.3 Simplified single-file format (optional)

If all writes in a batch go to same file (common case), optimize:
```
[rootKeyLen: u8] [rootKey: UTF-8]
[pathLen: u16 LE] [path: UTF-8]
[count: u32 LE]
for each write:
  [position: u64 LE]
  [dataLen: u32 LE] [data: bytes]
  [hashHex: 40 bytes]
  [callbackIdLen: u8] [callbackId: UTF-8]
```

Saves ~20 bytes per piece (rootKey + path overhead).

---

## Phase 5: Cleanup & Polish

### 5.1 Feature flag

- Add to ConfigHub: `experimental.batchedHttpWrites`
- Default: false (until stable)

### 5.2 Metrics & logging

- Batch size histogram
- Flush trigger breakdown (size vs timeout vs explicit)
- Per-batch latency (POST → last ACK)

### 5.3 Error handling

- Partial batch failure: some pieces succeed, some fail
- Connection loss mid-batch: timeout and reject pending promises
- Server restart: detect via WebSocket close, fail pending batches

---

## Risk Assessment

| Risk | Mitigation |
|------|------------|
| WebSocket connection lost mid-batch | Timeout + reject promises, fall back to HTTP writes |
| Hash mismatch in batch | Individual piece fails, others succeed (current behavior) |
| Memory pressure from large batches | Buffer pooling, configurable batch size |
| Complexity vs benefit | Benchmark at each phase, abandon if gains insufficient |
| Multiple clients receive broadcast | Clients filter by callbackId (only originator has pending promise) |
| Results delivered before JS ready | JS registers pendingWrites before POST, so always ready |

---

## Success Criteria

1. **Phase 1 complete:** Kotlin endpoint accepts batch, sends WebSocket ACKs
2. **Phase 2 complete:** JS batching works with unit tests passing
3. **Phase 3 complete:** Benchmark shows >2x throughput improvement
4. **Phase 4 complete:** Benchmark shows >3x improvement, stable under load
5. **Phase 5 complete:** Feature flagged, metrics in place, error handling robust

---

## Open Questions

1. ~~Batch results: one frame vs stream individual~~ → Stream individual (simpler)
2. ~~HTTP→WebSocket wiring~~ → Shared queue + broadcast to all clients (single-user scenario)
3. Should batch timeout be adaptive based on throughput?
4. Do we need backpressure if batches queue up faster than disk can write?
5. Should we support mixed-file batches or optimize for single-file only?
