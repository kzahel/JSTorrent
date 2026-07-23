# ChromeOS WebSocket Write Bottleneck Investigation

**Date:** 2025-01-29

## Problem

With `USE_WEBSOCKET_WRITES = true` on ChromeOS, disk write throughput is limited to ~10 MB/s despite:
- Network download capacity of 55 MB/s
- Android standalone achieving 45+ MB/s disk writes
- Disk queue configured with 8 workers (even 800 doesn't help)

## Architecture

```
Chrome Extension (Service Worker)
    ↓ WebSocket (dedicated write connection)
Android Companion Server (java-websocket library)
    ↓ Dispatchers.IO coroutines
SAF FileManager (Storage Access Framework)
```

## Investigation

### 1. Initial Hypothesis: Companion Read Loop Blocking

The companion's WebSocket read loop was:
```kotlin
while (true) {
    val data = session.receive() ?: break
    handleMessage(data)  // Awaited
}
```

Even though `handleFileWrite` does `scope.launch(Dispatchers.IO)` (fire-and-forget), we suspected head-of-line blocking.

**Test:** Changed to `scope.launch { handleMessage(data) }` (fire-and-forget)

**Result:** No improvement. `concurrent=1 (max=1)` still.

### 2. Companion Stats Analysis

```
WS_WRITE: 10.1 MB/s, 51 writes, concurrent=1 (max=1), framesInFlight=0 (max=1), avg: total=20ms queue=0ms hash=2ms disk=18ms
```

- `concurrent=1 (max=1)` - Only 1 write coroutine ever runs at a time
- `framesInFlight=0 (max=1)` - Only 1 frame in flight between arrival and ACK
- `disk=18ms` - SAF writes are fast enough

This means frames are **arriving one at a time**, not that processing is slow.

### 3. Extension Side: WebSocket Backpressure

Added instrumentation to `DaemonConnection.sendFrame()`:
```typescript
const buffered = this.ws.bufferedAmount
// Log: avg=7356049, max=8011211
```

**7-8 MB buffered** in the browser's WebSocket send buffer. The extension is sending frames, but they're piling up waiting to transmit.

### 4. Companion Side: Frame Arrival Rate

Added instrumentation to `JavaWebSocketSession.onMessage()`:
```
RECV RATE: 10.0 MB/s, 50 frames, queueDepth max=1, dropped=no
```

- Frames arrive at java-websocket callback at **10 MB/s**
- `queueDepth max=1` - Channel is NOT backed up, reader is fast
- Frames are simply arriving slowly

## Root Cause

The bottleneck is the **WebSocket transport layer** between Chrome and Android on ChromeOS:

1. Extension sends 1MB frames via `ws.send()` - non-blocking, buffers internally
2. Browser's WebSocket buffer fills up (7-8 MB)
3. Data transmits over Chrome↔Android network bridge at ~10 MB/s
4. Companion receives frames at 10 MB/s
5. Each frame processed in 20ms, but next frame doesn't arrive for ~100ms

The serialization happens at the network level, not in the code.

## What's NOT the Bottleneck

- Companion read loop (made it non-blocking, no change)
- Kotlin Channel capacity (queueDepth max=1, not backed up)
- Dispatchers.IO thread pool (plenty of threads available)
- SAF write speed (18ms per 1MB write = 55 MB/s theoretical)
- Disk queue workers (8 or 800 makes no difference)

## Possible Causes for 10 MB/s Limit

1. **ChromeOS Chrome↔Android network bridge** - May have bandwidth limits
2. **java-websocket library** - May be slow at receiving/parsing large binary frames
3. **WebSocket protocol overhead** - Framing/masking for 1MB messages

## Potential Solutions to Investigate

1. **Try Netty WebSocket server** - `NettyWebSocketServer.kt` exists in codebase, may be faster
2. **Reduce frame size** - Smaller frames might have less overhead
3. **Different IPC mechanism** - Unix sockets, shared memory, or Android's Binder
4. **Profile java-websocket** - Check if it's CPU-bound on frame parsing
5. **Check ChromeOS network config** - Is there a bridge MTU or bandwidth limit?

## Code Changes Made During Investigation

1. `IoWebSocketHandler.kt` line 181: `scope.launch { handleMessage(data) }` (non-blocking read loop)
2. `daemon-connection.ts`: Added `bufferedAmount` stats logging
3. `JavaWebSocketSession.kt`: Added `RECV RATE` stats logging

## Comparison: Android Standalone

Android standalone achieves 45+ MB/s because:
- No WebSocket - direct FFI calls to Kotlin
- No Chrome↔Android network bridge
- Uses `file://` URIs with pooled `RandomAccessFile` handles (not SAF)
