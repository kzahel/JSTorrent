# WebSocket Writes for ChromeOS Companion

## Problem Statement

HTTP POST writes are responsible for ~50% of throughput loss in ChromeOS companion mode:

| Mode | Throughput |
|------|------------|
| With HTTP writes | 17 MB/s |
| Null storage | 30 MB/s |
| Target (standalone) | 60 MB/s |

The HTTP path has high overhead:
- `recv` time is 33-127ms for 1MB chunks (only 8-30 MB/s effective)
- Competes with WebSocket for Chrome's networking resources
- Each write requires full HTTP round-trip

## Proposed Solution

Add file write support to the existing IO WebSocket protocol. This eliminates HTTP overhead and enables fire-and-forget writes.

## Protocol Design

### New Opcodes

Add to `Protocol.kt` and `daemon-connection.ts`:

```
OP_FILE_WRITE       = 0x30  // Extension → Companion
OP_FILE_WRITE_ACK   = 0x31  // Companion → Extension (optional)
OP_FILE_WRITE_ERROR = 0x32  // Companion → Extension (on failure)
```

### Message Format

**OP_FILE_WRITE (Extension → Companion):**
```
[envelope:8][root_key:4 LE][offset:8 LE][data:N]

- envelope: standard 8-byte header (version, opcode, flags, requestId)
- root_key: file handle identifier (matches existing HTTP write API)
- offset: byte offset within file (u64 LE)
- data: piece data to write (variable length, typically 16KB-1MB)
```

**OP_FILE_WRITE_ACK (Companion → Extension):**
```
[envelope:8][root_key:4 LE][offset:8 LE][status:1]

- status: 0 = success, non-zero = error code
```

**OP_FILE_WRITE_ERROR (Companion → Extension):**
```
[envelope:8][root_key:4 LE][offset:8 LE][error_code:4 LE][message:N]
```

### Fire-and-Forget Mode

For maximum throughput, the extension can send writes without waiting for ACKs:

1. Set `requestId = 0` in envelope to indicate no response needed
2. Companion writes data and only sends `OP_FILE_WRITE_ERROR` on failure
3. Extension tracks pending writes and retries on disconnect

This matches how TCP_SEND works today (no per-send acknowledgment).

## Implementation Plan

### Phase 1: Companion Side (Kotlin)

**Files to modify:**

1. `android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt`
   - Add new opcodes to `Protocol` object
   - Add to `IO_OPCODES` set

2. `android/companion-server/src/main/java/com/jstorrent/companion/server/IoWebSocketHandler.kt`
   - Add `handleFileWrite()` method
   - Wire up in `handlePostAuth()` switch
   - Reuse existing file handle registry from `FileRoutes.kt`

3. `android/companion-server/src/main/java/com/jstorrent/companion/server/FileRoutes.kt`
   - Extract file writing logic into shared function
   - Make `FileHandleRegistry` accessible from `IoWebSocketHandler`

**Implementation sketch:**
```kotlin
private fun handleFileWrite(requestId: Int, payload: ByteArray) {
    if (payload.size < 12) return  // root_key(4) + offset(8) minimum

    val rootKey = payload.getUIntLE(0)
    val offset = payload.getLongLE(4)
    val data = payload.copyOfRange(12, payload.size)

    val handle = fileHandleRegistry.get(rootKey)
    if (handle == null) {
        sendFileWriteError(requestId, rootKey, offset, "Handle not found")
        return
    }

    scope.launch(Dispatchers.IO) {
        try {
            handle.write(offset, data)
            if (requestId != 0) {
                sendFileWriteAck(requestId, rootKey, offset)
            }
        } catch (e: Exception) {
            sendFileWriteError(requestId, rootKey, offset, e.message ?: "Write failed")
        }
    }
}
```

### Phase 2: Extension Side (TypeScript)

**Files to modify:**

1. `packages/engine/src/adapters/daemon/daemon-connection.ts`
   - Add opcodes constants
   - No new methods needed (uses existing `sendFrame`)

2. `packages/engine/src/adapters/daemon/daemon-filesystem.ts`
   - Modify `DaemonFileHandle.write()` to use WebSocket instead of HTTP
   - Add write queue for fire-and-forget mode
   - Handle `OP_FILE_WRITE_ERROR` responses

**Current HTTP write path:**
```typescript
// daemon-file-handle.ts
async write(position: number, data: Uint8Array): Promise<number> {
  await this.connection.requestWithHeaders(
    'POST',
    `/write/${this.rootKey}`,
    { 'X-JST-Offset': String(position) },
    data,
  )
  return data.length
}
```

**New WebSocket write path:**
```typescript
async write(position: number, data: Uint8Array): Promise<number> {
  // Build frame: [envelope:8][root_key:4][offset:8][data:N]
  const frame = new ArrayBuffer(8 + 4 + 8 + data.length)
  const view = new DataView(frame)

  // Envelope
  view.setUint8(0, PROTOCOL_VERSION)
  view.setUint8(1, OP_FILE_WRITE)
  view.setUint16(2, 0, true)  // flags
  view.setUint32(4, 0, true)  // requestId=0 for fire-and-forget

  // Payload
  view.setUint32(8, this.rootKey, true)
  view.setBigUint64(12, BigInt(position), true)
  new Uint8Array(frame, 20).set(data)

  this.connection.sendFrame(frame)
  return data.length
}
```

### Phase 3: Error Handling

1. **Write errors**: Companion sends `OP_FILE_WRITE_ERROR`, extension logs and may retry
2. **Connection loss**: Extension tracks pending writes, retries on reconnect
3. **Backpressure**: Monitor WebSocket queue depth, pause writes if backing up

## Testing Plan

### Unit Tests

1. Protocol parsing tests for new opcodes
2. `IoWebSocketHandler` write handling tests
3. `DaemonFileHandle` WebSocket write tests

### Integration Tests

1. Write a piece via WebSocket, verify file contents
2. Concurrent writes from multiple pieces
3. Error handling: invalid handle, disk full, etc.
4. Reconnection: pending writes survive disconnect

### Performance Tests

1. **Throughput comparison**: HTTP vs WebSocket writes
   - Target: >30 MB/s with WebSocket writes (matching null-storage baseline)

2. **Latency measurement**: Time from write call to disk sync
   - Add instrumentation similar to existing HTTP write timing

3. **Full download test**: ChromeOS extension with WebSocket writes
   - Compare to current 17 MB/s baseline
   - Target: 30+ MB/s (closing the HTTP write gap)

## Rollout Strategy

1. **Feature flag**: Add `useWebSocketWrites` setting (default: false initially)
2. **A/B testing**: Compare throughput with HTTP vs WS writes
3. **Gradual rollout**: Enable by default once validated
4. **Deprecation**: Remove HTTP write path after WebSocket path is stable

## Future Optimizations

### Batched Writes

Combine multiple small writes into single WebSocket frame:
```
[envelope:8][count:4 LE]
  [root_key:4][offset:8][len:4][data:len]
  [root_key:4][offset:8][len:4][data:len]
  ...
```

### Write Coalescing

Buffer writes in companion and flush periodically:
- Reduces syscall overhead
- Enables better disk I/O patterns
- Risk: data loss on crash (mitigate with periodic fsync)

### Zero-Copy Path

If WebSocket library supports it, write directly from network buffer to file without intermediate copies.

## Success Metrics

| Metric | Current | Target |
|--------|---------|--------|
| Download throughput (ChromeOS) | 17 MB/s | 30+ MB/s |
| Write latency (p50) | ~70ms | <20ms |
| Write latency (p99) | ~150ms | <50ms |

## Open Questions

1. **Ordering guarantees**: Do we need writes to be ordered? Currently HTTP writes can complete out-of-order. WebSocket preserves order within connection.

2. **Flow control**: Should companion signal backpressure to extension? Current HTTP path implicitly rate-limits via response time.

3. **Reliability**: Is fire-and-forget acceptable? BitTorrent can re-download corrupted pieces, so losing a write is recoverable but wasteful.

## References

- [chromeos-companion-throughput.md](../chromeos-companion-throughput.md) - Investigation that identified HTTP write overhead
- `IoWebSocketHandler.kt` - Existing WebSocket protocol implementation
- `FileRoutes.kt` - Existing HTTP write implementation
- `daemon-file-handle.ts` - Extension-side file handle abstraction
