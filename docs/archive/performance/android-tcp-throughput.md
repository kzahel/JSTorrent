# Android TCP Throughput Investigation

## Problem Statement

Android native app achieves ~45 MB/s download speed vs ~90 MB/s on desktop (Chrome extension + Rust io-daemon) from the same seeder on LAN.

## Investigation Summary (2025-01-29)

### What We Ruled Out

| Suspect | Finding | Why Ruled Out |
|---------|---------|---------------|
| FFI overhead | 7 µs per crossing | See `docs/ffi-crossing-cost.md` - essentially free |
| Pipeline depth | 1500 blocks (24MB) | Plenty of headroom, not request-limited |
| TCP kernel buffers | 2-8 MB available | `/proc/sys/net/ipv4/tcp_rmem` shows adequate max |
| JS tick loop | Queue depth = 0 | JS draining queue as fast as data arrives |
| Tick batching | N/A | Data not backing up in queue |

### Root Cause: Socket Buffer Configuration

The Kotlin TCP read loop was configured with small buffers:

```kotlin
// TcpSocketService.kt - BEFORE
private const val RECEIVE_BUFFER_SIZE = 256 * 1024  // 256KB (kernel doubles to 512KB)

// TcpConnection.kt - BEFORE
private const val READ_BUFFER_SIZE = 128 * 1024     // 128KB per read
```

**Effect**: Small SO_RCVBUF caused TCP flow control to throttle the sender. The kernel buffer filled up between read() calls, closing the TCP window.

### The Fix

```kotlin
// TcpSocketService.kt - AFTER
private const val RECEIVE_BUFFER_SIZE = 2 * 1024 * 1024  // 2MB (kernel gives 4MB)

// TcpConnection.kt - AFTER
private const val READ_BUFFER_SIZE = 512 * 1024          // 512KB per read
```

### Results

| Metric | Before | After | Change |
|--------|--------|-------|--------|
| SO_RCVBUF | 512 KB | 4 MB | 8x |
| READ_BUFFER | 128 KB | 512 KB | 4x |
| TCP recv rate | 48 MB/s | **68 MB/s** | **+42%** |
| Reads/sec | 770 | 1205 | +56% |
| Avg read size | 69 KB | 58 KB | -16% |

## Remaining Gap

Still ~22 MB/s short of desktop's 90 MB/s. Potential causes:

### 1. `buffer.copyOf()` Allocation Pressure

Every read allocates a new ByteArray:
```kotlin
onData(buffer.copyOf(bytesRead))  // 1200 reads/s × 58KB = 70MB/s allocations
```

**Potential fix**: Buffer pool to reuse allocations.

### 2. Java InputStream 128KB Cap

Even with 512KB READ_BUFFER, max actual read is 128KB:
```
max=131072 bytes/read
```

This appears to be a Java/Android InputStream limitation.

**Potential fix**: Use NIO SocketChannel instead of InputStream.

### 3. Blocking I/O vs Async

Desktop uses Rust async I/O:
```rust
read_half.read(&mut buf).await  // Zero-copy, no thread blocking
```

Android uses blocking InputStream on coroutine:
```kotlin
input.read(buffer)  // Blocks thread, coroutine overhead
```

**Potential fix**: Migrate to NIO with Selector, or use Ktor/OkHttp NIO layers.

## Key Diagnostic Commands

```bash
# TCP socket buffer limits
adb shell cat /proc/sys/net/ipv4/tcp_rmem

# Watch TCP read stats (requires instrumentation in TcpConnection.kt)
adb logcat -s TcpConnection:I TcpBindings:I

# Key metrics to watch:
# - "TCP recv: X MB/s (raw)" - actual socket throughput
# - "pending: N events" - queue backup (should be 0-1)
# - "reads/s, MB/s, avg/min/max bytes/read" - read loop efficiency
```

## Architecture Overview

```
Seeder → TCP → Kernel buffer (SO_RCVBUF) → read() → ByteArray copy → Queue → JS tick → Process
                     ↑                         ↑           ↑
              TCP flow control          128KB cap    Allocation pressure
              throttles sender          per read     (GC)
```

## Files Changed

- `android/io-core/src/main/java/com/jstorrent/io/socket/TcpSocketService.kt` - RECEIVE_BUFFER_SIZE
- `android/io-core/src/main/java/com/jstorrent/io/socket/TcpConnection.kt` - READ_BUFFER_SIZE, logging

## NIO Migration (2025-01-29)

Migrated from InputStream to NIO SocketChannel for plain TCP connections.

### Changes

| Component | Before | After |
|-----------|--------|-------|
| Read API | `InputStream.read(byte[])` | `SocketChannel.read(ByteBuffer)` |
| Buffer type | Heap ByteArray | Direct ByteBuffer (off-heap) |
| Max read size | 128KB (Java InputStream limit) | 1MB (no cap) |
| TLS connections | SSLSocket + InputStream | SSLSocket + InputStream (unchanged) |

### Files Changed

- `TcpConnectionNio.kt` - New NIO-based connection handler with direct ByteBuffers
- `TcpConnectionBase.kt` - Common interface for both connection types
- `TcpSocketService.kt` - Uses SocketChannel for connects, dispatches to appropriate connection type
- `TcpConnection.kt` - Implements TcpConnectionBase (kept for TLS and server sockets)

### Results

| Metric | Before NIO | After NIO | Change |
|--------|------------|-----------|--------|
| TCP recv rate | 68 MB/s | **93.65 MB/s** | **+38%** |
| Desktop target | 90 MB/s | 90 MB/s | **Exceeded** |

Batch processing stats at steady state:
- 1335 flushes in 5s (~267 flushes/sec)
- 5.0 events/flush average
- 359.9 KB/flush average
- Queue depth: 2-3 events (JS keeping up)

### Why It Worked

1. **No 128KB read cap** - SocketChannel can read up to buffer size (1MB)
2. **Reduced GC pressure** - Direct ByteBuffer is off-heap, doesn't pressure GC
3. **Larger reads per syscall** - Fewer context switches

### Still Allocating (but not a bottleneck)

The `ByteArray(bytesRead)` allocation still happens in TcpConnectionNio for queuing to JS.
Buffer pooling could reduce GC pressure further but is no longer needed for throughput.

## Summary

| Phase | Change | Throughput |
|-------|--------|------------|
| Initial | Small buffers | 48 MB/s |
| Buffer tuning | 2MB SO_RCVBUF, 512KB read buffer | 68 MB/s |
| NIO migration | SocketChannel + direct ByteBuffer | **93.65 MB/s** |

Android now matches or exceeds desktop throughput (90 MB/s target).

## ChromeOS Companion Throughput (2025-01-29)

### Problem Statement

ChromeOS extension + companion app achieves ~10-17 MB/s vs ~93 MB/s standalone. Same NIO code, same buffer settings.

### Root Cause: ARCVM Kernel Limit

ChromeOS runs Android in ARCVM with a restricted kernel configuration:

```bash
# On ChromeOS ARCVM:
$ adb shell cat /proc/sys/net/core/rmem_max
262144  # 256KB max - cannot be increased without root

# On real Android devices:
$ adb shell cat /proc/sys/net/core/rmem_max
2097152  # 2MB+ - allows our 2MB SO_RCVBUF request
```

The `rmem_max` kernel parameter caps all socket receive buffers. Our 2MB request gets silently reduced to 256KB.

### Investigation Results

**WebSocket is NOT the bottleneck:**

| Metric | Value |
|--------|-------|
| TCP recv | 16-18 MB/s |
| WS send | 16-18 MB/s (matches TCP) |
| WS avgSend | 100-150µs/frame |
| Queue depth | 0-7 (no backlog) |

The WebSocket forwarding adds only ~100-150µs latency per frame. Data flows through without queuing.

**Actual bottleneck**: TCP window limited by 256KB kernel buffer cap.

### Throughput Comparison

| Environment | rmem_max | SO_RCVBUF | TCP Throughput |
|-------------|----------|-----------|----------------|
| Real Android | 2MB+ | 4MB (2MB×2) | **93 MB/s** |
| ChromeOS ARCVM | 256KB | 256KB (capped) | **16-18 MB/s** |

The ~5x throughput difference roughly matches the ~8x buffer difference (TCP window scaling effects).

### Diagnostic Commands (ChromeOS)

```bash
# Check kernel buffer limits
ssh chromeroot "adb shell cat /proc/sys/net/core/rmem_max"
ssh chromeroot "adb shell cat /proc/sys/net/ipv4/tcp_rmem"

# Watch throughput metrics
ssh chromeroot "adb logcat -s IoWebSocketHandler:I TcpConnectionNio:I"

# Key log lines:
# "WS THROUGHPUT: send=X.X MB/s" - WebSocket send rate
# "Socket N: X.X reads/s, Y.Y MB/s" - TCP read rate
# "SO_RCVBUF=262144" - actual buffer (should be 2MB but capped)
```

### Potential Optimizations

1. **Multiple peer connections** - Each peer gets its own 256KB buffer. More peers = more aggregate buffer.

2. **Reduce extension-side latency** - Faster block processing means faster ACKs, larger effective window.

3. **Request rmem_max increase from ChromeOS team** - This is the only real fix for the kernel limit.

### Files Changed

- `IoWebSocketHandler.kt` - Added throughput metrics (WS THROUGHPUT logging)
- `TcpConnectionNio.kt` - Already had SO_RCVBUF logging

## Future Optimization (Optional)

1. **Buffer pooling** - Pool ByteArrays to reduce GC pressure (not needed for throughput)
2. **Profile GC** - Verify allocation pressure is acceptable under load
3. **NIO for server sockets** - Currently use classic I/O for accepted connections
