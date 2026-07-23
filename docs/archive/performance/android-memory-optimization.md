# Android Memory Optimization

**Status:** Exploratory / idea backlog
**Date:** 2026-03-06
**Superseded by:** [android-memory-measurement-plan.md](android-memory-measurement-plan.md), [android-memory-performance-tuning.md](android-memory-performance-tuning.md)

## Note

This document is not the current implementation plan.

It captures possible fixes, hypotheses, and design ideas for Android memory
pressure handling, but it was written before we had enough runtime visibility to
justify specific interventions.

The authoritative document for current work is
[android-memory-measurement-plan.md](android-memory-measurement-plan.md). That
plan is intentionally narrower: instrument the current system, collect raw data,
and use measured behavior to decide what optimization work is actually needed.

For the current tuning reference, landed standalone fixes, and concrete
companion-mode follow-ups, see
[android-memory-performance-tuning.md](android-memory-performance-tuning.md).

## Problem

Downloading large torrents (2GB+) on Android causes the app to be killed by the Low Memory Killer (LMK), especially when the user multitasks. The app has been optimized for throughput but lacks memory visibility and pressure response.

## Current State

### What We Have

- **Foreground service** with `FOREGROUND_SERVICE_TYPE_DATA_SYNC` and `START_STICKY`
- **Engine memory limits** (in `ActivePieceManager`):
  - `maxActivePieces`: 128 (native), 10000 (desktop)
  - `maxBufferedBytes`: 64MB (native), 256MB (desktop)
- **PieceBufferPool**: Reuses `Uint8Array` buffers, capped at 16MB pool size
- **Swarm pruning** on completion: removes non-connected peers
- **TCP backpressure** at 16MB high water mark (pauses native TCP reads)
- **Known OOM comment** in code: "android standalone goes OOM near the end of a download (e.g. when in endgame) if maxActivePieces is too high"

### What We're Missing

1. **No `onTrimMemory()` callback** — app ignores Android's memory pressure warnings entirely
2. **No `JS_SetMemoryLimit()`** — QuickJS heap can grow without bound
3. **No manual `JS_RunGC()` calls** — relying on QuickJS auto-GC (threshold starts at 256KB, doubles)
4. **No memory profiling** — no way to see where memory is going at runtime
5. **No dynamic limit adjustment** — all limits are static constants, set at construction time, never change
6. **No piece eviction** — can't shed active pieces mid-download; can only reject new ones at the gate
7. **No swarm cap during download** — peers Map grows unbounded from trackers/DHT/PEX; only pruned on completion
8. **Transient JNI allocations** — `ByteArray` copies from JS→JVM for every piece write pile up under load

## How Android Kills Processes

LMK ranks by process importance and kills the biggest memory consumers first.

Foreground service = priority 3 (decent, but not immune). When user switches apps, the foreground app (priority 1) wins the RAM fight.

`onTrimMemory()` delivers escalating warnings:

| Level | Value | Meaning |
|-------|-------|---------|
| `RUNNING_MODERATE` | 5 | System getting low, not killable yet |
| `RUNNING_LOW` | 10 | Getting serious, free stuff or become a target |
| `RUNNING_CRITICAL` | 15 | About to start killing, last chance |
| `UI_HIDDEN` | 20 | UI gone, release UI resources |
| `BACKGROUND` | 40 | In the background kill list |
| `MODERATE` | 60 | Middle of the kill list |
| `COMPLETE` | 80 | Next to be killed |

**Not responding doesn't directly trigger the kill** — LMK checks actual RSS, not whether you handled the callback. But if you ignore warnings and stay fat, you're the obvious target. Freeing memory moves you down the kill list.

## Memory Budget Analysis

Rough breakdown of where memory goes during a large download:

| Component | Worst Case | Notes |
|-----------|-----------|-------|
| QuickJS heap baseline | ~10-20MB | Runtime, parsed code, closures, etc. |
| Active piece buffers | up to 256MB | 128 pieces × 2MB each (worst case) |
| PieceBufferPool | up to 16MB | Recycled buffers waiting for reuse |
| Peer connection state | ~5-10MB | 200 peers × metadata, buffers, bitfields |
| Swarm known peers | ~2-5MB | Address lists for all known peers (uncapped) |
| DHT routing + peer store | ~2-5MB | Up to 10K infohashes × 100 peers each |
| JNI transient copies | variable | ByteArray allocations for piece writes |
| Kotlin/JVM overhead | ~20-30MB | ART runtime, app classes, Compose UI |
| **Total** | **~300-350MB** | **On a device with 3-4GB shared with other apps** |

The piece buffers are the dominant cost. 128 active pieces × 2MB = 256MB is way too much for a phone.

## Implementation Plan

### Phase 1: Visibility (memory profiling)

Before optimizing, we need to see what's happening.

#### 1.1 Expose `JS_ComputeMemoryUsage()` via JNI

Add a native method to `QuickJsContext` that calls QuickJS's built-in memory stats function. Returns a structured breakdown of the JS heap: atoms, strings, objects, arrays, typed arrays, etc.

**Files:**
- `android/quickjs-engine/src/main/cpp/quickjs-jni.c` — add `nativeComputeMemoryUsage()`
- `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/QuickJsContext.kt` — add Kotlin binding

#### 1.2 Engine `getMemoryStats()` method

Collect existing scattered counters into one call:
- `activePieceManager`: active count, buffered bytes, peak buffered bytes
- `pieceBufferPool`: pool size, pool bytes, hit/miss ratio
- `swarm`: connected peers, known peers
- `dht`: routing table size, peer store size
- `connectionManager`: open connections, pending connections

**Files:**
- `packages/engine/src/core/bt-engine.ts` — add `getMemoryStats()`
- Wire through to native adapter bindings

#### 1.3 Debug manhole `memory` command

`adb shell am broadcast -a com.jstorrent.DEBUG --es cmd memory`

Output:
```
[QuickJS Heap]
  total: 142MB (malloc: 138MB, atoms: 2MB, strings: 1.5MB)
  objects: 45K, arrays: 12K, typed_arrays: 890

[Engine]
  active_pieces: 87/128, buffered: 52MB/64MB (peak: 61MB)
  buffer_pool: 8 buffers (14MB), hit_rate: 78%
  peers: 45 connected, 1200 known
  dht: 280 nodes, 3 infohashes

[Android]
  jvm_heap: 24MB/48MB (max: 256MB)
  native_heap: 198MB
  trim_level: RUNNING_LOW (last: 12s ago)
```

**Files:**
- `android/app/src/main/java/com/jstorrent/app/debug/DebugReceiver.kt` — add `memory` command

#### 1.4 Periodic memory logging

Every 30 seconds while downloading, log a single line:

```
[MEM] js:142M native:198M jvm:24M pieces:87/128 buf:52/64M peers:45 pool:14M
```

Visible in `emu logs` or `adb logcat -s JSTorrent-Mem`.

### Phase 2: Dynamic limits and memory pressure response

This is the core of the fix. Today all limits are static constants set at construction. They need to become dynamic — adjustable at runtime in response to memory pressure and recoverable when pressure drops.

#### 2.1 Make `ActivePieceManager` limits mutable

Currently `config.maxActivePieces` and `config.maxBufferedBytes` are set once in the constructor and never change. We need:

```typescript
// active-piece-manager.ts

// Replace static config with dynamic limits
private _effectiveMaxActivePieces: number
private _effectiveMaxBufferedBytes: number
private readonly _baselineMaxActivePieces: number  // original value, for recovery
private readonly _baselineMaxBufferedBytes: number

setLimits(maxActivePieces: number, maxBufferedBytes: number): void {
  this._effectiveMaxActivePieces = maxActivePieces
  this._effectiveMaxBufferedBytes = maxBufferedBytes
  // If we're now OVER the new limit, we need to evict
  if (this.activeCount > maxActivePieces || this.totalBufferedBytes > maxBufferedBytes) {
    this.evictToFitLimits()
  }
}

restoreLimits(): void {
  this._effectiveMaxActivePieces = this._baselineMaxActivePieces
  this._effectiveMaxBufferedBytes = this._baselineMaxBufferedBytes
  // No eviction needed — we're relaxing limits
}
```

`getOrCreate()` already checks these limits before creating pieces — it just needs to read from the `_effective*` fields instead of `config.*`.

#### 2.2 Piece eviction: `evictToFitLimits()`

This is new functionality. Today we can only reject new pieces at the gate (`getOrCreate` returns null). We need to actively throw away in-progress pieces to free memory.

**Eviction priority (least-valuable pieces evicted first):**

1. **Partial pieces with 0 blocks received** — no data downloaded yet, cheapest to lose
2. **Partial pieces with fewest blocks received** — least progress, sorted ascending
3. **FullyRequested pieces with fewest blocks received** — have outstanding requests but least data
4. **Never evict fullyResponded pieces** — these are complete and waiting for disk write, evicting them wastes all the downloaded data

```typescript
// active-piece-manager.ts

evictToFitLimits(): number {
  let evicted = 0

  // Build eviction candidates sorted by value (least valuable first)
  const candidates: ActivePiece[] = []

  // Tier 1: partial pieces, sorted by blocks received ascending
  const partials = [...this._partialPieces.values()]
  partials.sort((a, b) => a.blocksReceived - b.blocksReceived)
  candidates.push(...partials)

  // Tier 2: fullyRequested pieces, sorted by blocks received ascending
  const fullyReq = [...this._fullyRequestedPieces.values()]
  fullyReq.sort((a, b) => a.blocksReceived - b.blocksReceived)
  candidates.push(...fullyReq)

  // Never touch fullyResponded — they're complete, just waiting for disk

  for (const piece of candidates) {
    if (this.activeCount <= this._effectiveMaxActivePieces &&
        this.totalBufferedBytes <= this._effectiveMaxBufferedBytes) {
      break  // we're within limits now
    }
    this.logger.info(`EVICT piece ${piece.index} (${piece.blocksReceived}/${piece.blocksNeeded} blocks, ${piece.bufferedBytes} bytes)`)
    this.remove(piece.index)  // releases buffer back to pool, clears piece
    evicted++
  }

  // Also clear the buffer pool — evicted buffers went to the pool, but we need
  // them actually freed, not sitting in the pool
  if (evicted > 0 && this.bufferPool) {
    this.bufferPool.clear()
  }

  return evicted
}
```

**What happens to evicted pieces:**
- The piece data is lost (blocks already downloaded are discarded)
- The piece goes back to "not started" state from the torrent's perspective
- Peers still have outstanding requests for blocks of this piece — when responses arrive, they'll be dropped because the `ActivePiece` no longer exists (this is already handled — `torrent.ts` checks `activePieces.get(pieceIndex)` before processing)
- The piece will be re-requested in a future tick when limits allow
- CANCEL messages should be sent to peers for the evicted piece's outstanding requests

#### 2.3 Send CANCELs for evicted pieces

When a piece is evicted, we need to tell peers to stop sending data for it. Otherwise they waste bandwidth sending blocks we'll just drop.

```typescript
// torrent.ts or a new method on Torrent

cancelEvictedPiece(piece: ActivePiece): void {
  // For each connected peer, if they had outstanding requests for this piece,
  // send CANCEL messages for each pending block
  for (const peer of this._swarm.connectedPeers()) {
    const pendingBlocks = piece.getRequestsForPeer(peer.id)
    for (const block of pendingBlocks) {
      peer.sendCancel(piece.index, block.offset, block.length)
    }
  }
}
```

This needs to happen BEFORE the piece is removed from the manager (while we still have the request tracking data).

#### 2.4 Swarm size cap during download

Currently the swarm `peers` Map grows unbounded. Add a cap:

```typescript
// swarm.ts

private static readonly MAX_KNOWN_PEERS = 500  // on native, could be higher on desktop

addPeer(address: PeerAddress): SwarmPeer | null {
  // ... existing logic ...

  // New: cap check
  if (this.peers.size >= Swarm.MAX_KNOWN_PEERS) {
    this.pruneIdlePeers()  // remove oldest idle/failed peers to make room
    if (this.peers.size >= Swarm.MAX_KNOWN_PEERS) {
      return null  // can't add more
    }
  }
}

pruneIdlePeers(): number {
  // Remove peers that are: idle (never connected), failed, or banned
  // Keep: connected, connecting
  // Sort by last-seen ascending, remove oldest first
  let pruned = 0
  for (const [key, peer] of this.peers) {
    if (peer.state === 'connected' || peer.state === 'connecting') continue
    this.peers.delete(key)
    pruned++
    if (this.peers.size < Swarm.MAX_KNOWN_PEERS * 0.8) break  // prune to 80%
  }
  return pruned
}
```

#### 2.5 Engine `reduceMemory(level)` — the central pressure response

This is the method the Android side calls via JNI when `onTrimMemory()` fires. It coordinates the response across all engine subsystems.

```typescript
// bt-engine.ts

enum MemoryPressureLevel {
  LIGHT = 1,     // TRIM_MEMORY_RUNNING_MODERATE
  MODERATE = 2,  // TRIM_MEMORY_RUNNING_LOW
  CRITICAL = 3,  // TRIM_MEMORY_RUNNING_CRITICAL or COMPLETE
}

private _memoryPressureLevel: MemoryPressureLevel = 0  // 0 = no pressure
private _memoryPressureTime: number = 0

reduceMemory(level: MemoryPressureLevel): MemoryReductionReport {
  this._memoryPressureLevel = level
  this._memoryPressureTime = Date.now()
  const report: MemoryReductionReport = { level, actions: [], bytesFreed: 0 }

  // ---- LIGHT (RUNNING_MODERATE) ----
  // Goal: Free easy stuff without affecting throughput

  // 1. Clear all buffer pools (buffers sitting idle in the pool)
  for (const torrent of this.activeTorrents()) {
    const poolBytes = torrent.activePieces?.clearBufferPool() ?? 0
    report.bytesFreed += poolBytes
  }
  report.actions.push('cleared buffer pools')

  // 2. Prune idle/failed peers from all swarms
  for (const torrent of this.activeTorrents()) {
    torrent.swarm.pruneIdlePeers()
  }
  report.actions.push('pruned idle swarm peers')

  if (level < MemoryPressureLevel.MODERATE) return report

  // ---- MODERATE (RUNNING_LOW) ----
  // Goal: Significantly reduce memory. Throughput will take a hit.

  // 3. Reduce active piece limits by 50%
  for (const torrent of this.activeTorrents()) {
    if (!torrent.activePieces) continue
    const currentMax = torrent.activePieces.effectiveMaxActivePieces
    const newMax = Math.max(8, Math.floor(currentMax / 2))
    const currentBufMax = torrent.activePieces.effectiveMaxBufferedBytes
    const newBufMax = Math.max(8 * 1024 * 1024, Math.floor(currentBufMax / 2))
    torrent.activePieces.setLimits(newMax, newBufMax)
    report.actions.push(`reduced pieces ${currentMax}->${newMax}, buf ${currentBufMax}->${newBufMax}`)
    // setLimits calls evictToFitLimits internally, which sends CANCELs
  }

  // 4. Reduce max peers per torrent
  for (const torrent of this.activeTorrents()) {
    const cm = torrent.connectionManager
    const currentMax = cm.maxPeersPerTorrent
    const newMax = Math.max(5, Math.floor(currentMax / 2))
    cm.setMaxPeersPerTorrent(newMax)  // need to add this method
    // This will cause maintenance to drop excess peers on next run
    report.actions.push(`reduced max peers ${currentMax}->${newMax}`)
  }

  // 5. Disable endgame mode temporarily (it creates duplicate requests = more memory)
  for (const torrent of this.activeTorrents()) {
    torrent.endgameManager?.setEnabled(false)
  }
  report.actions.push('disabled endgame mode')

  if (level < MemoryPressureLevel.CRITICAL) return report

  // ---- CRITICAL (RUNNING_CRITICAL / COMPLETE) ----
  // Goal: Survive at all costs. Pause everything except the most-progressed torrent.

  // 6. Pause all but the most-progressed torrent
  const activeDls = [...this.activeTorrents()]
    .filter(t => t.state === 'downloading')
    .sort((a, b) => b.progress - a.progress)

  if (activeDls.length > 1) {
    for (let i = 1; i < activeDls.length; i++) {
      activeDls[i].pause()  // fully stops network, destroys active pieces
      report.actions.push(`paused torrent ${activeDls[i].name}`)
    }
  }

  // 7. The surviving torrent gets minimal limits
  const survivor = activeDls[0]
  if (survivor?.activePieces) {
    survivor.activePieces.setLimits(4, 4 * 1024 * 1024)  // 4 pieces, 4MB buffer
    report.actions.push('survivor torrent reduced to 4 pieces / 4MB')
  }

  // 8. Close peers beyond top 5 fastest on the surviving torrent
  if (survivor) {
    const dropped = survivor.connectionManager.dropSlowestPeers(5)
    report.actions.push(`dropped ${dropped} slow peers, kept top 5`)
  }

  this.logger.warn(`[MEMORY] reduceMemory(${level}): ${report.actions.join(', ')}`)
  return report
}
```

#### 2.6 Recovery when pressure drops

Memory pressure isn't permanent — the user might close other apps, or the system frees memory. The engine needs to recover.

**Recovery is checked in the tick loop** — `doTick()` already runs every 1-100ms. Add a check:

```typescript
// bt-engine.ts, inside doTick()

// After checkBackpressure(), before torrent ticks:
this.checkMemoryPressureRecovery()

private checkMemoryPressureRecovery(): void {
  if (this._memoryPressureLevel === 0) return  // no pressure active

  // Don't recover too quickly — wait at least 30 seconds since last pressure signal
  if (Date.now() - this._memoryPressureTime < 30_000) return

  // Gradually recover: step down one level at a time
  this._memoryPressureLevel = Math.max(0, this._memoryPressureLevel - 1)

  if (this._memoryPressureLevel === 0) {
    // Fully recovered — restore all baseline limits
    for (const torrent of this.activeTorrents()) {
      torrent.activePieces?.restoreLimits()
      torrent.connectionManager?.restoreMaxPeers()
      torrent.endgameManager?.setEnabled(true)
    }
    // Resume paused torrents? Maybe not automatically — user might be confused.
    // Instead, just restore limits and let the queue manager handle resumption.
    this.logger.info('[MEMORY] Pressure recovered, restored baseline limits')
  } else {
    // Still under some pressure — re-apply the lower level
    this.reduceMemory(this._memoryPressureLevel)
    this.logger.info(`[MEMORY] Pressure eased to level ${this._memoryPressureLevel}`)
  }
}
```

**Important subtlety:** If Android sends `RUNNING_CRITICAL` then doesn't send anything else, we assume pressure persists. Recovery only happens when 30 seconds pass with no new trim callback. The Kotlin side resets `_memoryPressureTime` on every `onTrimMemory()` call.

#### 2.7 `onTrimMemory()` implementation (Kotlin side)

```kotlin
// JSTorrentApplication.kt

class JSTorrentApplication : Application(), ComponentCallbacks2 {

  private var lastTrimLevel: Int = 0
  private var lastTrimTime: Long = 0

  override fun onTrimMemory(level: Int) {
    super.onTrimMemory(level)
    lastTrimLevel = level
    lastTrimTime = System.currentTimeMillis()

    Log.w("JSTorrent-Mem", "onTrimMemory level=$level (${trimLevelName(level)})")

    val engineLevel = when {
      level >= TRIM_MEMORY_COMPLETE     -> 3  // critical
      level >= TRIM_MEMORY_RUNNING_CRITICAL -> 3
      level >= TRIM_MEMORY_RUNNING_LOW  -> 2  // moderate
      level >= TRIM_MEMORY_RUNNING_MODERATE -> 1  // light
      else -> 0
    }

    if (engineLevel > 0) {
      // Call into QuickJS on the JS thread
      engineController?.evaluateOnJsThread(
        "__jstorrent_memory_pressure($engineLevel)"
      )

      // Also trigger native GC from the Kotlin side
      quickJsContext?.runGC()
    }
  }

  // For the debug manhole memory command
  fun getLastTrimInfo(): Pair<Int, Long> = Pair(lastTrimLevel, lastTrimTime)
}
```

#### 2.8 Native binding for memory pressure

The Kotlin side calls into JS via the existing global function pattern:

```typescript
// packages/engine/src/adapters/native/native-engine-adapter.ts

// Register the callback during engine init
(globalThis as any).__jstorrent_memory_pressure = (level: number) => {
  engine.reduceMemory(level as MemoryPressureLevel)
}
```

#### 2.9 ConnectionManager: dynamic peer limits

Need to add methods to adjust peer limits at runtime:

```typescript
// connection-manager.ts

private _baselineMaxPeers: number
private _effectiveMaxPeers: number

setMaxPeersPerTorrent(max: number): void {
  this._effectiveMaxPeers = max
  // If we're over the new limit, drop excess peers on next maintenance
  // (maintenance already handles this — it checks available slots)
  // But we need to actively close excess connections:
  const excess = this.connectedCount - max
  if (excess > 0) {
    this.dropSlowestPeers(max)
  }
}

restoreMaxPeers(): void {
  this._effectiveMaxPeers = this._baselineMaxPeers
}

dropSlowestPeers(keepCount: number): number {
  const connected = this.getConnectedPeers()
  if (connected.length <= keepCount) return 0

  // Sort by download speed descending — keep the fastest
  connected.sort((a, b) => b.downloadSpeed - a.downloadSpeed)

  let dropped = 0
  for (let i = keepCount; i < connected.length; i++) {
    connected[i].close('memory_pressure')
    dropped++
  }
  return dropped
}
```

#### 2.10 Endgame manager: disable under pressure

Endgame sends duplicate requests for blocks that are already requested, which means more active data in flight, more memory. Under pressure it needs to stop.

```typescript
// endgame-manager.ts

private _enabled: boolean = true

setEnabled(enabled: boolean): void {
  this._enabled = enabled
  if (!enabled) {
    // Cancel all existing duplicate requests?
    // Probably not worth the complexity — just stop issuing new ones.
    // Existing duplicates will time out naturally.
  }
}

// In shouldEnterEndgame():
if (!this._enabled) return false
```

### Phase 3: Hard limits (safety nets)

#### 3.1 Set `JS_SetMemoryLimit()`

Cap QuickJS heap at **128MB**. When exceeded, JS allocations return null and QuickJS throws an InternalError. The engine should catch allocation failures gracefully — most importantly, `new Uint8Array(pieceLength)` in `PieceBufferPool.acquire()` or `ActivePiece` constructor.

```c
// quickjs-jni.c, in nativeCreate()
JS_SetMemoryLimit(rt, 128 * 1024 * 1024);
```

When this triggers, QuickJS throws `InternalError: out of memory`. We should:
- Catch this in `getOrCreate()` (wrapping the `new ActivePiece()` call)
- Log it loudly
- Trigger `reduceMemory(CRITICAL)` to proactively shed load
- NOT crash — just return null from `getOrCreate()`

#### 3.2 Expose `JS_RunGC()` via JNI

```c
// quickjs-jni.c
JNIEXPORT void JNICALL Java_...QuickJsContext_nativeRunGC(JNIEnv *env, jobject obj, jlong context_ptr) {
    JSRuntime *rt = JS_GetRuntime((JSContext *)context_ptr);
    JS_RunGC(rt);
}
```

Called from `onTrimMemory()` and after `reduceMemory()`. Also available via debug manhole.

#### 3.3 Lower default native limits

Current: 128 active pieces, 64MB buffered. Proposed:

| Setting | Current | Proposed | Rationale |
|---------|---------|----------|-----------|
| `maxActivePieces` | 128 | 48 | 48 × 2MB = 96MB worst case, more typical ~30-40MB |
| `maxBufferedBytes` | 64MB | 32MB | Tighter cap, rely on disk queue throughput |
| `maxPeersPerTorrent` | 20 | 20 | Keep (already reasonable) |
| `maxGlobalPeers` | 200 | 100 | Reduce connection state overhead |

Throughput impact should be minimal — the bottleneck on Android is usually disk I/O or network, not piece pipeline depth. Profile with phase 1 tools to validate before/after.

#### 3.4 `largeHeap=true` in manifest

Grants more Dalvik/ART heap. Doesn't help QuickJS (native heap) directly, but reduces pressure on the JVM side so the overall process footprint is less likely to trigger LMK.

**File:** `android/app/src/main/AndroidManifest.xml`

### Phase 3.5: Routine GC hygiene

Independent of memory pressure response — we should be doing this regardless.

Currently we **never** call `JS_RunGC()`. QuickJS auto-GC triggers when allocations since last GC exceed a threshold that starts at 256KB and doubles each time. Over a long download, the threshold grows huge and GC runs less frequently as the heap gets bigger — the opposite of what we want.

#### Why our heap is worse than libtorrent's

libtorrent uses `mmap` for file I/O by default. mmap'd pages are **file-backed** — the kernel can reclaim them under memory pressure by simply dropping the page (the data is on disk, re-read on next access). Our piece buffers are `Uint8Array` backed by `malloc` — **anonymous pages** that the kernel cannot reclaim without killing the process.

Same RSS, very different kill priority. A process with 200MB RSS that's 150MB file-backed + 50MB anonymous is far safer than one with 200MB that's all anonymous. Ours is almost entirely anonymous. Since we can't make our buffers reclaimable, we need to keep the heap tight.

#### Additionally: double JNI copy overhead

Every piece crosses the JNI boundary twice:
```
Kotlin TCP socket → JNI copy to ByteArray → JS callback → Uint8Array in QuickJS heap
→ piece assembly in JS → JNI copy back to ByteArray → Kotlin FileManager write
```
Each crossing allocates a transient `ByteArray` on the JVM side. Under load these pile up between GC runs. libtorrent does zero of this — peer buffer → C++ piece buffer → mmap write, all in one process.

#### When to trigger GC

| Trigger | Rationale |
|---------|-----------|
| After piece verification + disk write | Piece buffer released, block tracking / request maps / peer associations become garbage. Reclaim immediately instead of waiting for threshold. |
| Every 30 seconds during download | Floor interval. Catches anything that fell through cracks. Cost is proportional to live objects (mostly long-lived), not dead ones — cheap on a torrent client heap. |
| After large cleanup events | Torrent completion, peer disconnect wave, swarm prune — lots of newly-dead objects. |
| After `reduceMemory()` | Evicted pieces produce garbage. GC should run immediately after to actually free the native memory. |

#### Implementation

Expose `JS_RunGC()` via JNI (same as Phase 3.2), then call it from:
1. Kotlin-side periodic timer (30s interval, posted to JsThread)
2. Engine-side after piece completion (in `checkCompletion` path)
3. Engine-side after `reduceMemory()` returns

The periodic timer should be on the Kotlin side rather than JS `setInterval` so it runs even if the JS thread is busy/stuck. Post to the JsThread handler so it doesn't interrupt mid-tick.

### Phase 4: Smarter steady-state management

#### 4.1 Adaptive initial limits based on available memory

Instead of fixed limits, query `ActivityManager.getMemoryInfo()` at engine startup and set limits proportionally. Pass to engine at init:

```kotlin
val memInfo = ActivityManager.MemoryInfo()
activityManager.getMemoryInfo(memInfo)
val availMB = memInfo.availMem / (1024 * 1024)
val maxPieces = when {
    availMB > 1024 -> 48
    availMB > 512  -> 32
    availMB > 256  -> 16
    else           -> 8
}
val maxBufferedMB = when {
    availMB > 1024 -> 32
    availMB > 512  -> 16
    availMB > 256  -> 8
    else           -> 4
}
```

#### 4.2 Piece size awareness

A torrent with 2MB pieces costs 4× more per active piece than one with 512KB pieces. Scale `maxActivePieces` inversely with piece size:

```typescript
// When creating ActivePieceManager for a torrent
const pieceSizeMB = pieceLength / (1024 * 1024)
const scaleFactor = Math.max(0.25, 1 / pieceSizeMB)  // 2MB pieces → 0.5, 512KB → 2.0
const adjustedMax = Math.floor(baselineMaxActivePieces * scaleFactor)
```

This means a torrent with 512KB pieces can have more active pieces than one with 4MB pieces, keeping memory consumption roughly constant.

#### 4.3 Endgame mode memory cap

The existing code identifies endgame as the OOM trigger. In endgame, ALL remaining missing pieces become active simultaneously, and duplicate requests are issued. For a 2GB torrent at 95% with 2MB pieces, that's ~50 pieces × 2MB = ~100MB spike.

Changes needed:
- **Cap endgame active pieces** to `min(remaining, effectiveMaxActivePieces)` (already partially enforced by `getOrCreate`, but endgame bypasses some checks)
- **Limit duplicates more aggressively**: native already caps at 2 (vs 3 on V8), but under memory pressure, disable duplicates entirely (see 2.10)
- **Stagger endgame activation**: don't activate all remaining pieces at once — activate in batches of 8-16, letting each batch flush to disk before starting the next

#### 4.4 Tick-level memory monitoring

Add a lightweight memory check to the tick loop (no syscall, just check internal counters):

```typescript
// bt-engine.ts, inside doTick()

// Check if we're approaching our own limits before Android tells us
const totalBuffered = this.getTotalBufferedBytes()  // sum across torrents
const pctUsed = totalBuffered / this.totalMaxBufferedBytes
if (pctUsed > 0.9) {
  this.logger.warn(`[MEM] At ${Math.round(pctUsed * 100)}% of buffer capacity (${totalBuffered} bytes)`)
  // Proactive: slow down tick rate to let disk I/O drain
  this._tickDelayOverride = Math.max(this._tickDelayOverride ?? 0, 20)
}
```

## Data Flow: onTrimMemory → Engine Response

```
Android LMK detects pressure
         │
         ▼
ComponentCallbacks2.onTrimMemory(level)   [Kotlin, main thread]
         │
         ├──→ Log warning with level name
         ├──→ quickJsContext.runGC()       [JNI → JS_RunGC()]
         └──→ evaluateOnJsThread(          [posts to JsThread Handler]
               "__jstorrent_memory_pressure(level)")
                    │
                    ▼
              engine.reduceMemory(level)   [JS, on JsThread]
                    │
                    ├─ LIGHT: clear buffer pools, prune idle swarm peers
                    │
                    ├─ MODERATE: halve active piece limits → evictToFitLimits()
                    │    │                                      │
                    │    ├─ halve max peers → dropSlowestPeers()│
                    │    │                                      ▼
                    │    └─ disable endgame              send CANCELs for
                    │                                    evicted pieces
                    │
                    └─ CRITICAL: pause all but 1 torrent
                         │
                         ├─ survivor gets 4-piece / 4MB limit
                         └─ drop to 5 peers
                                │
                                ▼
                        Engine continues at reduced capacity
                        Recovery check runs every tick (30s cooldown)
                                │
                                ▼
                        If 30s pass with no new trim callback:
                        step down one pressure level → restore limits gradually
```

## Risks and Trade-offs

**Evicting pieces wastes bandwidth.** A partially downloaded piece that gets evicted means those blocks were downloaded for nothing. With 2MB pieces at 50% completion, that's 1MB wasted per eviction. Acceptable to survive vs. being killed.

**Recovery might be too aggressive.** If we restore limits to baseline after 30 seconds and then get immediately killed, we might want longer cooldown or partial recovery (e.g., restore to 75% of baseline instead of 100%).

**Pausing torrents under CRITICAL is user-visible.** The UI should show why a torrent was paused ("paused: memory pressure") so the user isn't confused. May need a new torrent state or status message.

**Endgame disabling slows completion.** The last few pieces take longer without duplicate requests. But at least the download completes instead of the app being killed at 98%.

**CANCEL storm on eviction.** Evicting 40 pieces could mean sending hundreds of CANCEL messages. This is a one-time burst and shouldn't be a problem, but worth watching.

## Testing Plan

1. **Baseline**: Download 2GB torrent with current code, monitor with new profiling tools, record memory timeline and crash point
2. **After phase 2**: Same download, verify trim memory callbacks fire and memory drops
3. **Multitask stress test**: Start 2GB download, switch between Chrome, YouTube, camera — verify survival
4. **Endgame test**: Download until ~95% complete, monitor memory spike in endgame mode
5. **Low-memory device**: Test on a 3GB RAM device (or use emulator with restricted memory)
6. **Eviction correctness**: Verify evicted pieces get re-downloaded, CANCELs are sent, no piece corruption
7. **Recovery test**: Trigger pressure, verify limits drop, close the other apps, verify limits recover after 30s

## Success Criteria

- 2GB download completes without being killed, even when multitasking
- Memory stays under 150MB total process RSS during download
- `adb shell am broadcast --es cmd memory` gives actionable diagnostics
- Periodic memory logs show stable or sawtooth pattern (GC working), not unbounded growth
- After memory pressure, engine recovers to normal throughput within 60 seconds
