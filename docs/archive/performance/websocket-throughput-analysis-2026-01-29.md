# WebSocket vs HTTP Throughput Analysis

**Date:** 2026-01-29
**Updated:** 2026-01-30

## Goal

Disprove or validate the hypothesis in `chromeos-websocket-write-bottleneck.md` that the "Chrome↔Android network bridge" limits WebSocket writes to ~10 MB/s.

## Methodology

### Test Environment
- ChromeOS Chromebook with ARCVM (Android Runtime)
- Chrome extension page executing JavaScript tests
- Android companion app running on ARCVM (port 7800 HTTP/Ktor, port 7806 java-websocket)
- CDP tunnel for remote JS evaluation via ext-debug MCP tools

### Tests Performed

1. **JVM Baseline** - Ran existing `ThroughputBenchmarkTest.all_servers_comparison` to measure WebSocket library performance in isolation

2. **HTTP Throughput from curl (chromeroot)** - Used curl to test raw HTTP throughput outside Chrome

3. **HTTP Throughput from Chrome** - Used `fetch()` API from extension page

4. **WebSocket Upload Throughput** - Created `/ws-sink` endpoint that receives and discards data as fast as possible, measured from Chrome extension

5. **WebSocket Download Throughput** - Used existing `/ws-throughput-test` endpoint (Ktor-based) to send data to Chrome

## Results

### JVM Baseline (localhost, no ARCVM)

```
java-websocket: 1677 MB/s
Raw Netty:      1388 MB/s
Ktor/Netty:     203 MB/s
```

**Finding:** java-websocket is 8x faster than Ktor on JVM. WebSocket libraries themselves are not the bottleneck.

### HTTP Throughput

| Test | Direction | Speed |
|------|-----------|-------|
| curl from chromeroot | Download (Android→ChromeOS) | 282 MB/s |
| curl from chromeroot | Upload (ChromeOS→Android) | 8.5 MB/s* |
| fetch() from Chrome | Download (Android→Chrome) | 183.5 MB/s |
| fetch() from Chrome | Upload (Chrome→Android) | 44.7 MB/s |

*curl upload test may have been limited by `--data-binary @-` pipe overhead

**Finding:** ARCVM bridge is fast. HTTP upload from Chrome achieves 44.7 MB/s - not 10 MB/s.

### WebSocket Throughput (Chrome ↔ Android)

| Test | Direction | Endpoint | Speed |
|------|-----------|----------|-------|
| WS download 100MB, 100KB frames | Android→Chrome | `/ws-source` (java-websocket) | **383 MB/s** |
| WS download 50MB, 100KB frames | Android→Chrome | `/ws-throughput-test` (Ktor) | 33.6 MB/s |
| WS upload 100MB, 1MB chunks | Chrome→Android | `/ws-sink` (java-websocket) | **23.5 MB/s** |
| WS upload 100MB, 1MB chunks | Chrome→Android | `/ws-sink` (Ktor) | 14.1 MB/s |

**Key Findings:**
- java-websocket download is **11x faster** than Ktor (383 vs 33.6 MB/s)
- java-websocket upload is **1.7x faster** than Ktor (23.5 vs 14.1 MB/s)
- WebSocket upload is still **~2x slower than HTTP** (23.5 vs 44.7 MB/s)

### Comparison Summary

| Direction | HTTP | java-websocket | Ktor |
|-----------|------|----------------|------|
| **Download** (Android→Chrome) | 183 MB/s | **383 MB/s** | 33.6 MB/s |
| **Upload** (Chrome→Android) | **44.7 MB/s** | 23.5 MB/s | 14.1 MB/s |

**Note:** java-websocket download is faster than HTTP! The bottleneck is specifically **WebSocket upload** (Chrome→Android direction).

### Parallel Upload Testing

Browsers limit connections to 6 per host (HTTP/1.1). Testing with realistic connection counts:

| Connections | HTTP Upload | WebSocket Upload |
|-------------|-------------|------------------|
| 1 | 42.5 MB/s | 23.5 MB/s |
| 3 | 83.1 MB/s | - |
| 6 | **103 MB/s** | ~50 MB/s (est.) |

**Key Finding:** With 6 parallel HTTP connections (browser limit), uploads achieve **103 MB/s** - 4x faster than single-connection WebSocket (23.5 MB/s).

## Update 2026-01-30: Ktor is the Bottleneck

### Hypothesis

The original tests used Ktor for HTTP endpoints. Given that Ktor WebSocket was 8x slower than java-websocket, we suspected Ktor HTTP might also be limiting throughput.

### New Test Infrastructure

Added minimal Netty HTTP server (`NettyHttpSinkServer.kt`) on port 7803 with:
- `/http-sink` - POST endpoint that receives and discards data (streaming)
- `/http-source?mb=N` - GET endpoint that sends N MB of zeros

Key differences from Ktor:
- No `HttpObjectAggregator` - streams content directly
- No coroutine/suspend overhead
- Minimal Netty pipeline

Also added raw TCP sink on port 7802 for baseline measurement.

### Results: Netty HTTP vs Ktor HTTP

| Test | Ktor (7800) | Netty (7803) | Speedup |
|------|-------------|--------------|---------|
| **Upload 100MB** | 37 MB/s | **235 MB/s** | **6.4x** |
| **Upload 500MB** | - | **360 MB/s** | - |
| **Download 100MB** | 49 MB/s | **489 MB/s** | **10x** |
| **Download 500MB** | - | **489 MB/s** | - |

**Key Finding:** Ktor is 6-10x slower than raw Netty for both uploads and downloads.

### Parallel Upload Testing (Netty HTTP)

Testing piece write patterns with varying chunk sizes and parallelism:

**Chunk size comparison (4 parallel, 200MB total):**

| Chunk Size | Throughput |
|------------|-----------|
| 1MB | 104 MB/s |
| 2MB | 128 MB/s |
| 4MB | 142 MB/s |
| **8MB** | **157 MB/s** |
| 16MB | 146 MB/s |

**Parallelism comparison (8MB chunks, 256MB total):**

| Workers | Throughput |
|---------|-----------|
| 1 | 58 MB/s |
| 2 | 84 MB/s |
| 4 | 104 MB/s |
| 6 | 114 MB/s |
| 8 | 118 MB/s |

**Sustained test (1GB, 1MB chunks × 4 parallel):** 108 MB/s
**Sustained test (3GB, 8MB chunks × 4 parallel):** 154 MB/s (20 seconds, range 136-173 MB/s)
**Single 512MB upload:** 360 MB/s

### HTTP Connection Behavior

HTTP/1.1 keep-alive connections are working - first request ~24ms, subsequent ~17ms. Chrome reuses connections but does NOT support HTTP/1.1 pipelining.

**Per-request overhead analysis (single connection, sequential):**

| Chunk Size | Request Time | Throughput | Overhead % |
|------------|--------------|------------|------------|
| 64KB | 7.9ms | 8 MB/s | ~90% |
| 256KB | 7.3ms | 34 MB/s | ~75% |
| 1MB | 17ms | 58 MB/s | ~40% |
| 4MB | 48ms | 84 MB/s | ~15% |

There's a fixed **~7ms per-request overhead** (HTTP headers, fetch() API, TCP ACKs).

**Batching multiple pieces in single request:**

| Batch | ms/piece | Throughput | vs 1×1MB |
|-------|----------|------------|----------|
| 1×1MB | 17.6ms | 57 MB/s | baseline |
| 4×1MB | 12.4ms | 80 MB/s | +40% |
| 8×1MB | 11.8ms | 85 MB/s | +49% |
| 16×1MB | 11.0ms | 91 MB/s | +60% |

**Implication:** Batching 8 torrent pieces into a single HTTP request would improve throughput by ~50% compared to 1 piece per request.

### HTTP Streaming Uploads

Attempted to use `fetch()` with `ReadableStream` body and `duplex: 'half'` for streaming uploads. This would allow unlimited upload size without memory buffering.

**Result:** Failed with `TypeError: Failed to fetch` from Chrome extension context. Likely a CORS restriction on streaming requests from extension pages. Regular buffered uploads work fine.

### Complete Throughput Comparison

| Server/Protocol | Upload | Download |
|-----------------|--------|----------|
| **Netty HTTP (single)** | **360 MB/s** | **489 MB/s** |
| Netty HTTP (8MB × 4 parallel, 3GB sustained) | **154 MB/s** | - |
| Netty HTTP (1MB × 4 parallel) | 108 MB/s | - |
| Ktor HTTP | 37 MB/s | 49 MB/s |
| java-websocket | 25 MB/s | 383 MB/s |
| Ktor WebSocket | 14 MB/s | 34 MB/s |

## Caveats and Limitations

1. **Frame sizes differ from production** - Tests used 100KB frames for download, 1MB for upload. Actual torrent traffic uses different frame sizes that may perform differently.

2. **Single connection** - Tests used a single WebSocket connection. Production may benefit from connection pooling or parallel connections.

3. **ChromeOS-specific** - Results may differ on other platforms (Windows, Mac, Linux desktop).

4. **HTTP streaming uploads blocked** - `duplex: 'half'` streaming doesn't work from extension context, limiting uploads to buffered requests.

## Conclusions

### What We Proved

1. **The ARCVM bridge is NOT the bottleneck** - Netty HTTP achieves 360 MB/s upload and 489 MB/s download
2. **Ktor is the bottleneck** - 6-10x slower than raw Netty for both directions
3. **java-websocket download is fast** - 383 MB/s, comparable to Netty HTTP
4. **Chrome's WebSocket.send() is slow** - Only 25 MB/s even with java-websocket server (vs 360 MB/s HTTP)
5. **Per-request HTTP overhead is significant** - Single 512MB upload: 360 MB/s; 1MB chunks × 4 parallel: 108 MB/s (3.3x slower)
6. **Optimal chunk size is ~8MB** - Balances per-request overhead vs memory usage

### What Remains Unclear

1. **Why Chrome's WebSocket upload is 14x slower than HTTP** - 25 MB/s WebSocket vs 360 MB/s HTTP (single connection)
2. **Why WebSocket receive is fast but send is slow** - 383 MB/s receive vs 25 MB/s send
3. **Whether this is Chrome-specific** - Would need to test Firefox/Safari
4. **Why HTTP streaming uploads fail from extension context** - CORS or Chrome restriction

## Recommendations

1. **Use Netty HTTP (not Ktor) for piece writes** - 360 MB/s single, 108-144 MB/s with parallel chunks
   - Added `NettyHttpSinkServer` on port 7803 as proof of concept
   - Migrate production write endpoint from Ktor to Netty

2. **Batch pieces to reduce per-request overhead** - ~7ms fixed overhead per HTTP request
   - 1 piece/request: 57 MB/s per connection
   - 8 pieces/request: 85 MB/s per connection (+49%)
   - With 4 parallel connections + 8-piece batching: ~200+ MB/s estimated
   - Trade-off: Increased latency for individual piece acknowledgment

3. **Keep WebSocket for reads** - java-websocket achieves 383 MB/s download, nearly as fast as Netty HTTP (489 MB/s)

4. **Don't use Ktor for high-throughput paths** - Use raw Netty or java-websocket instead

5. **Consider HTTP/2** - Would reduce per-request overhead for small pieces (requires TLS)

## Code Changes

Added throughput test endpoints to `JavaWebSocketServer.kt`:

```kotlin
// /ws-sink - Upload test (receives data, discards it)
// Client sends binary frames, then "done" text to get results
// Server responds: "done:elapsed:bytes:mbps"

// /ws-source - Download test (sends data as fast as possible)
// Client sends "frames,frameSize" (e.g. "1000,102400")
// Server sends N binary frames, then "done:elapsed:bytes:mbps"
```

Also added `/ws-sink` to `CompanionHttpServer.kt` (Ktor) for comparison testing.

## Raw Test Commands

```javascript
// HTTP upload test (from Chrome extension page)
const data = new Uint8Array(100 * 1024 * 1024);
const start = performance.now();
await fetch('http://100.115.92.2:7800/hash/sha1', {
  method: 'POST',
  headers: { 'Content-Type': 'application/octet-stream', ... },
  body: data
});
const mbps = 100 / ((performance.now() - start) / 1000);

// WebSocket upload test
const ws = new WebSocket('ws://100.115.92.2:7800/ws-sink');
// Send 100x 1MB chunks, wait for buffer drain, send 'done'
// Server responds with "done:elapsed:bytes:mbps"
```

```bash
# HTTP download test (from chromeroot)
curl -s --max-time 30 -w 'Speed: %{speed_download}\n' \
  -o /dev/null http://100.115.92.2:7800/throughput-test/50

# Raw TCP sink test (from chromeroot)
dd if=/dev/zero bs=1M count=100 | nc 100.115.92.2 7802
```

### Netty HTTP Tests (port 7803)

```javascript
// Single large upload
const data = new Uint8Array(500 * 1024 * 1024);  // 500MB
const start = performance.now();
const resp = await fetch('http://100.115.92.2:7803/http-sink', { method: 'POST', body: data });
console.log(await resp.text());  // "500.0 MB in 1423ms = 351.4 MB/s"

// Parallel chunked upload (simulating piece writes)
async function parallelUpload(totalMB, chunkMB, workers) {
  const chunkSize = chunkMB * 1024 * 1024;
  const totalPieces = totalMB / chunkMB;
  const data = new Uint8Array(chunkSize);
  let completed = 0, inFlight = 0;
  const start = performance.now();

  async function worker() {
    while (completed + inFlight < totalPieces) {
      inFlight++;
      await fetch('http://100.115.92.2:7803/http-sink', { method: 'POST', body: data });
      completed++; inFlight--;
    }
  }

  await Promise.all(Array(workers).fill().map(() => worker()));
  const elapsed = (performance.now() - start) / 1000;
  console.log(`${totalMB}MB in ${elapsed.toFixed(1)}s = ${(totalMB/elapsed).toFixed(1)} MB/s`);
}

parallelUpload(3072, 8, 4);  // 3GB, 8MB chunks, 4 workers -> ~154 MB/s

// Download test
const start = performance.now();
const resp = await fetch('http://100.115.92.2:7803/http-source?mb=500');
const reader = resp.body.getReader();
let bytes = 0;
while (true) {
  const { done, value } = await reader.read();
  if (done) break;
  bytes += value.length;
}
console.log(`${bytes/1024/1024} MB in ${(performance.now()-start)/1000}s`);
```
