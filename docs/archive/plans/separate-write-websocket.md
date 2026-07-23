# Separate WebSocket for File Writes

## Problem

Verified writes on ChromeOS are limited to ~10-20 MB/s while downloads achieve 60 MB/s.

**Root cause**: A single WebSocket connection (`/io` on port 7801) handles both:
- TCP socket recv data (high-throughput downloads)
- File write frames (piece storage)

This creates contention. Chrome's WebSocket implementation may not handle full-duplex at high throughput efficiently.

## Evidence

| Path | Throughput | Notes |
|------|------------|-------|
| Downloads (TCP recv) | 60 MB/s | Works well |
| HTTP writes (Ktor port 7800) | 20 MB/s | Slow Ktor server |
| WebSocket writes (same conn as reads) | 10 MB/s | Worse due to contention |

When `USE_WEBSOCKET_WRITES=true`, writes compete with TCP recv on the same connection and throughput drops.

## Solution

Create a **second `/io` WebSocket connection** dedicated to file writes:

```
Connection 1 (port 7801): TCP socket data only
Connection 2 (port 7801): File writes only
```

Both use the fast java-websocket server but don't compete.

## Relevant Code

### Client Side (TypeScript)

**`packages/client/src/engine-manager/chrome-extension-engine-manager.ts`**
- Lines 220-271: Creates single `DaemonConnection`, passes to `StorageRootManager`
- Need to create second connection for writes

**`packages/engine/src/adapters/daemon/daemon-filesystem.ts`**
- Lines 1-30: `DaemonFileSystem` constructor takes single `DaemonConnection`
- Need to accept optional `writeConnection`

**`packages/engine/src/adapters/daemon/daemon-file-handle.ts`**
- Lines 98-112: `DaemonFileHandle` constructor takes `connection`
- Lines 177-271: `writeViaWebSocket()` uses `this.connection.sendFrame()`
- Need to use separate write connection if available

**`packages/engine/src/adapters/daemon/daemon-connection.ts`**
- Lines 74-179: `connectWebSocket()` - handles auth handshake
- Second connection needs same auth flow

### Server Side (Kotlin)

**`android/companion-server/src/main/java/com/jstorrent/companion/server/websocket/JavaWebSocketServer.kt`**
- Handles `/io` endpoint on port 7801
- Already supports multiple concurrent connections
- No changes needed

**`android/companion-server/src/main/java/com/jstorrent/companion/server/IoWebSocketHandler.kt`**
- Line 297: Handles `OP_FILE_WRITE`
- Each connection gets its own handler instance
- No changes needed

## Implementation Notes

1. **Auth**: Both connections need to authenticate. Can reuse same credentials getter.

2. **Frame handler**: The write connection only needs to handle `OP_FILE_WRITE_ACK` and `OP_FILE_WRITE_ERROR`. It won't receive TCP data.

3. **Reconnect**: Both connections should handle reconnect independently.

4. **Fallback**: If write connection fails, could fall back to main connection or HTTP.

5. **Frame queuing**: Write connection doesn't need `drainPendingFrames()` batching - that's for TCP recv.

## Expected Result

With separate connections:
- Downloads: 60 MB/s (unchanged)
- Writes: Should approach 60 MB/s (no contention)

Combined throughput could reach 100+ MB/s if disk I/O permits.
