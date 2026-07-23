# ChromeOS Companion Throughput Investigation

## Problem Statement

ChromeOS extension + Android companion achieves ~17 MB/s download speed vs ~70 MB/s that Flud achieves on the same device from the same seeder.

**Test Setup:**
- ChromeOS Chromebook with ARCVM (Android Runtime)
- LAN seeder with libtorrent capable of 100+ MB/s
- WiFi connection (not the bottleneck - Flud proves 70 MB/s possible)
- Single peer, pipeline depth 500

## Investigation Summary (2025-01-29)

### What We Ruled Out

| Suspect | Finding | Why Ruled Out |
|---------|---------|---------------|
| ARCVM network bridge | HTTP test: **400 MB/s** | curl from ChromeOS host → Android companion |
| WebSocket send | avgSend: 155µs, queue depth 0-7 | Companion sends as fast as it receives |
| WebSocket queuing | No backlog | Queue depth never builds up |
| TCP buffer size | Autotuning works | Reads reach 1MB with autotuning |
| Kernel rmem_max | Not the bottleneck | Flud works with same kernel limits |

### Key Findings

#### 1. HTTP vs WebSocket Performance
```
HTTP (ARCVM → ChromeOS):  ~400 MB/s
WebSocket torrent data:   ~17 MB/s
```
The ARCVM bridge itself is fast. Something specific to our data path is slow.

#### 2. TCP Autotuning Discovery
- Setting SO_RCVBUF explicitly **disables** TCP autotuning
- Kernel default (43KB) + autotuning → reads up to 1MB
- Explicit 256KB setting → reads capped at 260KB
- **Recommendation**: Don't set SO_RCVBUF, let kernel autotune

#### 3. Throughput Metrics with Autotuning
```
TCP recv:    17 MB/s at 160 reads/sec, avg 110KB/read
WS send:     17 MB/s (matches TCP - no bottleneck here)
avgSend:     155µs per frame
Queue depth: 0-7 (no backlog)
```

The TCP recv rate IS the limiting factor - WebSocket keeps up perfectly.

### Architecture Comparison

**Flud (70 MB/s):**
```
Seeder → WiFi → ARCVM bridge → Flud app (processes locally)
```

**JSTorrent (17 MB/s):**
```
Seeder → WiFi → ARCVM bridge → companion → WebSocket → Chrome → extension
```

The extra WebSocket hop shouldn't matter - HTTP proves 400 MB/s is possible on that path.

### Remaining Hypotheses

#### 1. uTP vs TCP
Flud likely uses **uTP (UDP-based)** by default. Our engine is TCP-only.
- uTP might have different ARCVM network characteristics
- Different flow control behavior
- **To test**: Force Flud to TCP-only mode and compare

#### 2. Extension Block Request Rate
Even with pipeline depth 500, the extension might be slow to:
- Process incoming blocks
- Generate new block requests
- The round-trip through WebSocket adds latency

**Evidence needed**: Measure time from block received → next request sent

#### 3. BitTorrent Protocol Overhead
Our protocol implementation might have inefficiencies:
- Request message generation
- Piece verification
- State machine overhead

#### 4. Chrome WebSocket Receive Performance
The `avgSend=155µs` measures coroutine completion, not actual delivery.
Chrome's WebSocket receive might be slower than HTTP receive.

**To test**: WebSocket throughput test endpoint (added but needs client)

### Files Changed

- `IoWebSocketHandler.kt` - Added throughput metrics
- `TcpSocketService.kt` - Disabled explicit SO_RCVBUF for autotuning
- `CompanionHttpServer.kt` - Added HTTP and WS throughput test endpoints

### Diagnostic Commands

```bash
# Check HTTP throughput (ARCVM → ChromeOS)
ssh chromeroot "curl -s -w '%{speed_download}\n' -o /dev/null http://100.115.92.2:7800/throughput-test/50"

# Watch torrent throughput
ssh chromeroot "adb logcat -s IoWebSocketHandler:I TcpConnectionNio:I"

# Key metrics:
# "WS THROUGHPUT: send=X.X MB/s" - should match TCP recv
# "Socket N: X.X reads/s, Y.Y MB/s" - raw TCP from seeder
# "SO_RCVBUF=..." - should be small (~43KB) for autotuning
```

## Next Steps

### Short Term
1. **Test uTP hypothesis**: Check if Flud uses uTP, force TCP-only for comparison
2. **Measure extension latency**: Add timing for block recv → request send
3. **Test WebSocket throughput**: Create WS client to test raw WS speed

### Medium Term
4. **Implement uTP**: Would eliminate TCP path differences
5. **Profile extension processing**: Identify JS-side bottlenecks
6. **Optimize block request pipelining**: Reduce round-trip latency

### Long Term
7. **Move processing to companion**: Reduce WebSocket round-trips
8. **Native crypto in companion**: Offload piece hashing from extension

## Summary

| Component | Measured Speed | Bottleneck? |
|-----------|----------------|-------------|
| ARCVM bridge (HTTP) | 400 MB/s | No |
| TCP recv from seeder | 17 MB/s | Limited by downstream |
| WebSocket send | 17 MB/s | No (matches TCP) |
| SO_RCVBUF | 3 MB | No (plenty of headroom) |
| **Null storage mode** | **30 MB/s** | - |
| Standalone Android | 60 MB/s | - |
| Target (Flud) | 70 MB/s | - |

**Key insights**:
1. Standalone Android mode achieves 60 MB/s with the same TCP code
2. **HTTP POST writes cost ~13 MB/s** (17 → 30 MB/s with null storage)
3. Remaining 30 MB/s gap is WebSocket/Chrome/extension overhead

## Analysis: Standalone vs Companion (2025-01-29)

**Standalone (60 MB/s):**
```
Seeder → TCP → Android → QuickJS → Process
```

**Companion (17 MB/s):**
```
Seeder → TCP → Android → ByteArray alloc → copy → frame alloc → copy → WS → Chrome
```

The companion path had **double allocation and double copy** per TCP read:
1. TcpConnectionNio: `ByteArray(bytesRead)` + `directBuffer.get(data)`
2. IoWebSocketHandler: `ByteArray(frameSize)` + `System.arraycopy(data, ...)`

At 60 MB/s with 100KB reads = 600 reads/sec = 120 MB/s allocation pressure.

## Optimization: Zero-Copy Framing

Implemented single-allocation path:
1. TcpConnectionNio allocates `ByteArray(12 + bytesRead)` with header space
2. Copies data directly at offset 12: `directBuffer.get(frame, 12, bytesRead)`
3. IoWebSocketHandler fills in 12-byte header (no copy!)
4. Sends frame directly

**Result**: 1 allocation, 1 copy instead of 2 allocations, 2 copies.

### Files Changed

- `TcpSocketCallback.kt` - Added `onTcpDataFramed()` with default impl
- `TcpConnectionNio.kt` - Allocate with header space, call framed callback
- `TcpSocketService.kt` - Pass framed callback for NIO connections
- `IoWebSocketHandler.kt` - Implement `onTcpDataFramed()` for zero-copy send

### Timing Instrumentation Added

New metrics in logs:
- `copyTime=Xµs/read (Y% of time)` - TcpConnectionNio allocation+copy overhead
- `frameBuild=Xµs (Y%)` - IoWebSocketHandler frame build time

### Results (2025-01-29)

**Copy overhead is NOT the bottleneck:**
```
copyTime=219µs/read (3.6% of time)
frameBuild=20µs (0.3%)
```

96% of time is spent blocking on `channel.read()` - waiting for data from seeder.

| Metric | Before | After | Impact |
|--------|--------|-------|--------|
| Allocations/read | 2 | 1 | Reduced GC |
| Copies/read | 2 | 1 | 3.6% → ~2% |
| Throughput | 17 MB/s | 17-18 MB/s | Minimal |

**Key findings:**
- Pipeline is full (500 blocks) - request latency not the issue
- Extension CPU at 68% - not CPU-bound
- Seeder simply isn't sending faster

### TCP Buffer Check

Checked SO_RCVBUF during download:
```
rcvBuf=3072KB (3MB)
```

**TCP window is NOT the bottleneck.** With 3MB buffer and ~1ms RTT, theoretical max is 3GB/s.

## HTTP POST Write Interference (2025-01-29)

### Discovery

File writes go through HTTP POST `/write/{root_key}`. Instrumentation revealed:
```
WRITE: 1024KB in 97ms (recv=53ms, hash=2ms, write=41ms)
WRITE: 1024KB in 67ms (recv=47ms, hash=2ms, write=17ms)
WRITE: 1024KB in 152ms (recv=127ms, hash=2ms, write=22ms)
```

The `recv` time (receiving HTTP body from Chrome) is surprisingly high - 33-127ms for 1MB over localhost. This is only ~8-30 MB/s.

### Null Storage Test

Disabled piece writes (null storage mode):
```
With writes:    17 MB/s
Null storage:   30 MB/s  ← 1.8x improvement!
```

**HTTP POST writes are responsible for ~50% of throughput loss.**

### Architecture

Extension uses a disk queue with 6 workers for writes. Writes are parallelized and don't block the main event loop. However, HTTP POST uploads compete with WebSocket downloads for:
1. Chrome's network stack
2. ARCVM bridge bandwidth (bidirectional)
3. Ktor/Netty thread pool

### Remaining Gap

| Mode | Throughput | Gap |
|------|------------|-----|
| With writes | 17 MB/s | - |
| Null storage | 30 MB/s | - |
| Standalone target | 60 MB/s | 30 MB/s |

Even without writes, there's still a 30 MB/s gap to standalone. This suggests additional overhead in:
- Chrome WebSocket receive/dispatch
- Extension message processing
- V8 ↔ extension marshaling

### Potential Optimizations

**For write interference:**
1. Send writes over WebSocket instead of HTTP - eliminates HTTP overhead, reuses existing connection
2. Batch multiple pieces into single request
3. Fire-and-forget writes (skip waiting for response)

**For remaining gap:**
1. Profile Chrome WebSocket receive performance
2. Measure extension message handler latency
3. Consider moving more processing to companion (reduce message count)
