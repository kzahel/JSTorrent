# Multi-Peer Tick Overload: JS Thread Starvation on Real-World Torrents

**Date:** 2026-02-06
**Platform:** Android (Pixel 9), QuickJS engine
**Torrent:** Ubuntu Server 24.04.2 (~2.6 GB, 256KB pieces, public swarm)

## Summary

Downloading a popular torrent with many peers (15-20) causes the JS thread to become completely unresponsive. Ticks that should take 1-2ms instead take 1-8 **seconds**, leading to cascading failure: peers time out, downloads stall, and the app eventually OOMs. The same engine handles a single-peer LAN download at 90 MB/s with no issues.

## Observed Behavior

### Ubuntu torrent (20 peers, ~30 MB/s)

Screenshots showed progressive JS thread degradation:

| Time | Latency | Tick | Handler Q max | Peers | Active Pieces |
|------|---------|------|---------------|-------|---------------|
| 1:35 | 1.8s | 58ms | 10 | 14 | 293 |
| 1:36 | 7.0s | <1ms | 23 | 13 | 317 |
| 1:38 | **37.5s** | <1ms | **182** | 2 | 93 |

App eventually OOMed.

### Instrumented run (same torrent)

Logcat with added instrumentation revealed the root cause:

```
Tick: 6 ticks, avg 1152ms (js=740ms pump=413ms/max1284ms), max 3447ms, work=100%
  | 18 peers, 512 active
  | BLOCKS:recv=1063/sent=924, PIPE:74% of 5950
  | HandlerQ:1/10(tickMax=2), TCP:conn=0/close=1/sec=0

TCP batch: 8 flushes, avg 901.8 events/flush, avg 12567.2 KB/flush
Disk batch: 6 flushes, avg 59.2 events/flush
Batch write: 15.26 MB/s, 360 writes, avg 4ms (hash=1ms/14ms, disk=3ms/18ms)
```

Worst single tick observed:

```
Tick bottleneck: handlerQ=4, totalMs=7778 (js=7272 pump=506),
  pendingTcp=2275, pendingHash=0, pendingDisk=14
```

**A single tick took 7.8 seconds.** During that time, 2275 more TCP events accumulated.

### LAN download (1 peer, 90 MB/s) — baseline

```
Tick: 1327 ticks, avg 2.4ms (js=2.1ms pump=0.3ms/max15ms), max 49ms, work=95%
  | 1 peer, 10 active
  | BLOCKS:recv=16.6/sent=16.2

Backpressure: 9 active (1/1 partial, 7 fullyReq, 1 awaiting write),
  9.00MB buffered, PIPE:500/500, disk: 89.7MB/s

Batch write: 93.41 MB/s, 468 writes, avg 3ms
```

**3× higher throughput (90 vs 30 MB/s) with zero degradation.**

## Root Cause Analysis

### The vicious cycle

```
Many peers → scattered blocks across many pieces
  → 512 active pieces (fullyResponded pieces pile up awaiting hash+write)
  → each tick processes ALL accumulated TCP data (~14 MB, ~1000 events)
  → tick takes seconds (js=2-7s, pump=0.5-1.5s)
  → more TCP data accumulates during slow tick
  → next tick is even bigger
  → peers time out, downloads stall
  → 512 pieces × 256KB = 128MB+ in piece buffers → OOM
```

### Why it doesn't happen with 1 peer

With a single peer, data arrives sequentially. Only ~10 pieces are active at once because:
- One peer fills pieces one at a time
- Pieces complete and get verified/written before new ones are needed
- Pipeline depth = 500 blocks, but all targeted at a few pieces

With 20 peers, each peer requests different pieces. All 20 peers fill different pieces simultaneously, creating hundreds of partially-complete pieces that all stay in memory.

### Key bottlenecks identified

1. **Unbounded active pieces** (`maxActivePieces: 10000`). The `fullyResponded` count (awaiting hash verification + disk write) grows without bound. These pieces hold their full data buffers in memory.

2. **Unbounded TCP batch flush**. `drainAndPackTcpBatch()` drains ALL pending events in one shot. At 14-22 MB per flush and ~1000 events, the JS thread blocks for seconds parsing this data.

3. **Unbounded job pump**. `executeAllPendingJobs()` processes ALL pending Promise microtasks synchronously. With hundreds of pieces completing, this means 500-1700 Promise chain steps per pump (piece finalization: assemble → sha1 → writePieceVerified).

4. **Disk write throughput drops**. LAN: 93 MB/s sequential. Ubuntu: 5-15 MB/s scattered. With 512 pieces writing to different file offsets, the disk can't keep up, which makes the fullyResponded backlog grow even faster.

### What the handler Q actually shows

The handler Q max of 182 (from the initial un-instrumented run) is NOT from TCP data callbacks flooding the queue — TCP data uses the batched `ConcurrentLinkedQueue` path, not `jsThread.post()`. The handler Q fills because:
- The tick monopolizes the JS thread for seconds
- Connection callbacks (connect/close) from I/O threads can't be processed
- Job pump runnables chain through the handler queue
- Health check runnables get delayed (explaining the 37.5s latency measurement)

## Potential Solutions

### 1. Cap active pieces on native (highest impact, simplest)

Reduce `maxActivePieces` from 10000 to something proportional to peer count. The LAN test shows 10 active pieces is enough for 90 MB/s. A reasonable cap:

```typescript
// In active-piece-manager.ts defaults:
maxActivePieces: isNativeRuntime ? 64 : 10000
```

This bounds memory (64 × 256KB = 16MB) and bounds tick work. Pieces awaiting verification/write (fullyResponded) would count against this cap, creating natural backpressure.

**Trade-off**: Peers may idle if all their available pieces are at capacity. This is acceptable — it's better than OOM.

### 2. Cap TCP batch flush size per tick

Instead of draining ALL events in `drainAndPackTcpBatch()`, cap to a maximum (e.g. 4MB or 500 events) and process the rest next tick:

```kotlin
fun drainAndPackTcpBatch(maxEvents: Int = 500, maxBytes: Int = 4 * 1024 * 1024): ByteArray? {
    val batch = mutableListOf<TcpDataEvent>()
    var totalBytes = 0
    while (totalBytes < maxBytes && batch.size < maxEvents) {
        val event = pendingTcpData.poll() ?: break
        batch.add(event)
        totalBytes += event.data.size
    }
    // ... rest unchanged
}
```

This keeps each tick bounded to ~4MB of data processing, spreading work across ticks. Remaining data stays in the ConcurrentLinkedQueue for the next tick.

**Trade-off**: Increases latency between data arrival and processing. Not meaningful in practice since the current situation is seconds of latency anyway.

### 3. Bound executeAllPendingJobs per tick

Instead of pumping ALL jobs after each tick, cap the number of jobs per pump cycle:

```kotlin
// In tick runnable:
val pumpStart = System.currentTimeMillis()
eng.context.pumpJobsBatched(maxJobs = 200)  // Already exists!
val pumpEnd = System.currentTimeMillis()
```

The `pumpJobsBatched()` method already exists in `QuickJsContext`. Remaining jobs would be processed in subsequent tick cycles.

**Trade-off**: Piece finalization takes more ticks to complete. Acceptable since hash+write are already async.

### 4. Activate backpressure based on active piece count

When `fullyResponded` count exceeds a threshold, pause TCP reads:

```typescript
// In checkBackpressure():
const pendingPieces = this.activePieces?.fullyRespondedCount ?? 0
if (pendingPieces > 32) {
    this.socketFactory.setBackpressure?.(true)
}
```

This prevents new data from arriving while the engine is still processing completed pieces.

### 5. Prioritize piece completion over new pieces (longer term)

Request blocks for nearly-complete pieces before starting new ones. The `shouldPrioritizePartials()` logic exists but only limits PARTIAL pieces (not fullyRequested/fullyResponded). Extending this to consider total active count would reduce scatter.

## Recommended Approach

Apply fixes 1 + 2 together:
- **Cap active pieces to 64 on native** — bounds memory and tick work
- **Cap TCP batch flush to 500 events / 4MB** — bounds per-tick JS processing time

These two changes address both the memory issue (OOM) and the tick duration issue (seconds per tick). They're low-risk since the LAN test proves the engine works well with small active piece counts.

Fix 3 (bounded job pump) is a good follow-up but less critical once active pieces are capped, since fewer pieces = fewer Promise chains.

## Instrumentation Added

The following diagnostics were added during this investigation and should be kept:

- **EngineController tick stats**: `HandlerQ:current/max(tickMax=N), TCP:conn=N/close=N/sec=N`
- **Tick bottleneck warning**: Logs when `handlerQ > 50` or `pumpMs > 200` with breakdown of pending TCP/hash/disk counts
- **TcpBindings callback counters**: Tracks connect/close/secured callback rates
- **QuickJsContext**: `executeAllPendingJobs` logs at WARN when >100 jobs or >50ms

## Data Reference

Raw logcat from instrumented Ubuntu download (2026-02-06 15:05-15:06):

```
15:05:38 Tick bottleneck: totalMs=1985 (js=1780 pump=205), pendingTcp=2277
15:05:39 Tick bottleneck: totalMs=585  (js=84 pump=501),   pendingTcp=1293, pendingDisk=61
15:05:42 Tick bottleneck: totalMs=3447 (js=2163 pump=1284), pendingTcp=1845
15:05:42 Tick: 6 ticks, avg 1152ms, max 3447ms | 18 peers, 512 active
15:05:43 Tick bottleneck: totalMs=439  (js=31 pump=408),   pendingTcp=1416, pendingDisk=54
15:05:46 Tick bottleneck: totalMs=2927 (js=2713 pump=214), pendingTcp=2178
15:05:46 Tick bottleneck: totalMs=330  (js=109 pump=221),  pendingTcp=843, pendingDisk=135
15:05:49 Tick bottleneck: totalMs=2826 (js=2313 pump=513), pendingTcp=1215
15:05:49 Tick: 6 ticks, avg 1166ms, max 2927ms | 19 peers, 512 active
15:05:51 Tick bottleneck: totalMs=511  (js=51 pump=460),   pendingTcp=970, pendingDisk=40
15:05:55 Tick: 3 ticks, avg 1751ms, max 3742ms | 20 peers, 512 active
15:05:58 Tick bottleneck: totalMs=1640 (js=150 pump=1490), pendingTcp=2002, pendingDisk=108
15:06:06 Tick bottleneck: totalMs=7778 (js=7272 pump=506), pendingTcp=2275
15:06:06 Tick: 3 ticks, avg 3553ms, max 7778ms | 20 peers, 512 active
```
