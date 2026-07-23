# UI Subscription Architecture

## Problem Statement

The current UI data flow has several issues:

1. **RPC polling adds queue pressure** - When viewing torrent details (peers, files, trackers), Kotlin makes RPC calls into JS for each data type. These calls wait in the JS thread queue, competing with data callbacks.

2. **Stale data during high load** - When JS thread latency is high (2s+), RPC responses are delayed, causing UI to show stale data.

3. **No load adaptation** - State pushes happen every 500ms regardless of JS thread load. UI polling continues even when the engine is overloaded.

4. **Ad-hoc architecture** - Multiple uncoordinated mechanisms: state push interval, RPC calls, tracker polling loop in ViewModel.

### Current Data Flow

```
JS Engine                          Kotlin/UI
    │                                  │
    ├──[500ms interval]────────────────┤  State push (torrents, piece diffs)
    │                                  │
    │◄─────[RPC: getPeers]─────────────┤  UI calls into JS
    ├─────[response]───────────────────►│  JS returns data
    │                                  │
    │◄─────[RPC: getFiles]─────────────┤  Another RPC call
    ├─────[response]───────────────────►│
    │                                  │
    │  (RPCs wait in JS thread queue)  │
```

## Proposed Architecture

### Push-only model with subscriptions

UI subscribes to data it needs. Engine pushes all subscribed data in a single payload. No RPC calls for detail data.

```
JS Engine                          Kotlin/UI
    │                                  │
    │◄─────[subscribe: peers, files]───┤  UI declares interest
    │                                  │
    ├──[push: state + peers + files]───►│  Engine pushes everything
    │                                  │
    ├──[push: state + peers + files]───►│  Next push cycle
    │                                  │
    │◄─────[unsubscribe: peers]────────┤  UI navigates away
    │                                  │
    ├──[push: state + files]───────────►│  Only subscribed data
```

### Benefits

1. **Zero RPC queue pressure** - No calls waiting in JS thread queue
2. **Batched updates** - All data in one JSON payload, one FFI crossing
3. **Natural backpressure** - If JS is slow, pushes are less frequent
4. **Portable** - Subscription logic in JS, works for Android/iOS/desktop

## Design Decisions

1. **Single global subscriber** - One UI per engine. No subscriber IDs. Future remote UI would get its own manager instance.

2. **Base state is a subscription type** - The torrent list summary is NOT always pushed. UI must explicitly subscribe to 'state' type with hash '_global'. This prevents pushing list data when only viewing torrent details. Piece-level data (bitfield, changes, active states) is per-torrent via the 'pieces' subscription.

3. **Most recent subscription controls frequency** - Last `subscribe()` call sets the push interval for everything. In practice each view subscribes to a single type, so this is straightforward.

4. **Immediate first push** - `subscribe()` restarts the push loop immediately so data arrives without waiting for the next interval.

5. **Pause/resume instead of TTL** - No time-based expiration. Kotlin calls `pause()` when screen not visible, `resume()` when visible.

6. **Auto-cleanup on torrent removal** - If a torrent is removed, any subscriptions for it are automatically cleaned up.

7. **Load adaptation via tick drift** - (Future) Engine measures time between ticks to detect overload. Can slow push frequency when behind.

## Design

### Subscription Manager (JS)

```typescript
// packages/engine/src/adapters/native/subscriptions.ts

/**
 * Subscription types:
 *
 * 'state' (hash: '_global') - Torrent list summary:
 *   - torrents: TorrentSummary[] (name, progress %, speed, state)
 *
 * Per-torrent types (hash: infohash hex):
 *   'peers'    - Connected peers (address, client, up/down speed, flags)
 *   'files'    - File list with completion percentage
 *   'trackers' - Tracker status (url, announce result, peer count)
 *   'pieces'   - Piece map, recent changes, active download states
 *   'details'  - Extended torrent info (creation date, comment, etc.)
 */
type SubscriptionType = 'state' | 'peers' | 'files' | 'trackers' | 'pieces' | 'details'

export class SubscriptionManager {
  private subs = new Map<string, Set<SubscriptionType>>()  // hash -> types
  private paused = false
  private pushInterval = 500
  private loopTimeout: ReturnType<typeof setTimeout> | null = null
  private engine: BtEngine
  private onPush: (payload: string) => void

  constructor(engine: BtEngine, onPush: (payload: string) => void) {
    this.engine = engine
    this.onPush = onPush

    // Auto-cleanup when torrent removed
    engine.on('torrent-removed', (torrent) => {
      const hash = toHex(torrent.infoHash)
      if (this.subs.has(hash)) {
        console.warn(`Cleaning up subscriptions for removed torrent ${hash}`)
        this.subs.delete(hash)
      }
    })
  }

  /**
   * Subscribe to data for a torrent (or 'state' for base torrent list).
   *
   * For 'state' subscription, hash should be '_global'.
   * Restarts push loop immediately for fast first update.
   */
  subscribe(type: SubscriptionType, hash: string, intervalMs: number): void {
    let types = this.subs.get(hash)
    if (!types) {
      types = new Set()
      this.subs.set(hash, types)
    }
    types.add(type)
    this.pushInterval = intervalMs
    this.restartLoop()
  }

  /**
   * Unsubscribe from specific data type.
   */
  unsubscribe(type: SubscriptionType, hash: string): void {
    const types = this.subs.get(hash)
    if (types) {
      types.delete(type)
      if (types.size === 0) this.subs.delete(hash)
    }
  }

  /**
   * Unsubscribe all for a torrent (when navigating away from detail view).
   */
  unsubscribeAll(hash: string): void {
    this.subs.delete(hash)
  }

  /**
   * Pause all pushes (screen not visible).
   */
  pause(): void {
    this.paused = true
    if (this.loopTimeout) {
      clearTimeout(this.loopTimeout)
      this.loopTimeout = null
    }
  }

  /**
   * Resume pushes (screen visible again).
   */
  resume(): void {
    this.paused = false
    this.restartLoop()
  }

  /**
   * Clear all subscriptions.
   */
  clear(): void {
    this.subs.clear()
    this.paused = false
    if (this.loopTimeout) {
      clearTimeout(this.loopTimeout)
      this.loopTimeout = null
    }
  }

  /**
   * Check if any subscriptions exist.
   */
  hasSubscriptions(): boolean {
    return this.subs.size > 0
  }

  private restartLoop(): void {
    if (this.loopTimeout) {
      clearTimeout(this.loopTimeout)
    }
    if (!this.paused) {
      this.loop()
    }
  }

  private loop(): void {
    if (this.paused) return

    const payload = this.buildPayload()
    this.onPush(JSON.stringify(payload))

    this.loopTimeout = setTimeout(() => this.loop(), this.pushInterval)
  }

  private buildPayload(): StatePayload {
    const payload: StatePayload = {}

    // Include torrent list only if subscribed to 'state'
    const globalSubs = this.subs.get('_global')
    if (globalSubs?.has('state')) {
      payload.torrents = this.buildTorrentSummaries()
    }

    // Add per-torrent subscribed data
    for (const [hash, types] of this.subs) {
      if (hash === '_global') continue
      for (const type of types) {
        const data = this.getData(type, hash)
        if (data !== null) {
          payload[type] ??= {}
          payload[type]![hash] = data
        }
      }
    }

    return payload
  }

  private getData(type: SubscriptionType, hash: string): unknown {
    const torrent = this.engine.getTorrentByHash(hash)
    if (!torrent) return null

    switch (type) {
      case 'peers':
        return this.getPeersData(torrent)
      case 'files':
        return this.getFilesData(torrent)
      case 'trackers':
        return this.getTrackersData(torrent)
      case 'pieces':
        return this.getPiecesData(torrent)
      case 'details':
        return this.getDetailsData(torrent)
      default:
        return null
    }
  }

  // Data extraction methods (move existing logic from controller.ts)
  private buildTorrentSummaries(): TorrentSummary[] { /* ... */ }
  private getPeersData(torrent: Torrent): PeerInfo[] { /* ... */ }
  private getFilesData(torrent: Torrent): FileInfo[] { /* ... */ }
  private getTrackersData(torrent: Torrent): TrackerInfo[] { /* ... */ }
  private getPiecesData(torrent: Torrent): PiecesData { /* bitfield + changes + active */ }
  private getDetailsData(torrent: Torrent): TorrentDetails { /* ... */ }
}
```

### Push Loop

The push loop is integrated into `SubscriptionManager`. Key behaviors:

- Uses `setTimeout` chain instead of `setInterval` for natural self-throttling
- Restarts immediately on `subscribe()` for fast first update
- Stops when `pause()` called, resumes on `resume()`
- Interval controlled by most recent `subscribe()` call

```typescript
private restartLoop(): void {
  if (this.loopTimeout) {
    clearTimeout(this.loopTimeout)
  }
  if (!this.paused) {
    this.loop()  // Run immediately
  }
}

private loop(): void {
  if (this.paused) return

  const payload = this.buildPayload()
  this.onPush(JSON.stringify(payload))

  // setTimeout chain - next push after interval
  this.loopTimeout = setTimeout(() => this.loop(), this.pushInterval)
}
```

### Native Bindings

Expose subscription API to Kotlin:

```typescript
// Register global functions for Kotlin to call

globalThis.__jstorrent_subscribe = (type: string, hash: string, intervalMs: number) => {
  subscriptions.subscribe(type as SubscriptionType, hash, intervalMs)
}

globalThis.__jstorrent_unsubscribe = (type: string, hash: string) => {
  subscriptions.unsubscribe(type as SubscriptionType, hash)
}

globalThis.__jstorrent_unsubscribe_all = (hash: string) => {
  subscriptions.unsubscribeAll(hash)
}

globalThis.__jstorrent_pause_subscriptions = () => {
  subscriptions.pause()
}

globalThis.__jstorrent_resume_subscriptions = () => {
  subscriptions.resume()
}
```

### Kotlin Integration

```kotlin
// EngineController.kt - expose subscription methods

fun subscribe(type: String, hash: String, intervalMs: Int) {
    jsThread.post {
        // Note: QuickJS expects number, so pass as Int (FFI handles conversion)
        ctx.callGlobalFunction("__jstorrent_subscribe", type, hash, intervalMs)
    }
}

fun unsubscribe(type: String, hash: String) {
    jsThread.post {
        ctx.callGlobalFunction("__jstorrent_unsubscribe", type, hash)
    }
}

fun unsubscribeAll(hash: String) {
    jsThread.post {
        ctx.callGlobalFunction("__jstorrent_unsubscribe_all", hash)
    }
}

fun pauseSubscriptions() {
    jsThread.post {
        ctx.callGlobalFunction("__jstorrent_pause_subscriptions")
    }
}

fun resumeSubscriptions() {
    jsThread.post {
        ctx.callGlobalFunction("__jstorrent_resume_subscriptions")
    }
}
```

### ViewModel Usage

```kotlin
// TorrentListViewModel.kt

class TorrentListViewModel(...) : ViewModel() {

    init {
        // Subscribe to torrent list summary (name, progress %, speed, state)
        // '_global' is a special hash for non-torrent-specific data
        repository.subscribe("state", "_global", intervalMs = 500)
    }

    fun onScreenPaused() {
        repository.pauseSubscriptions()
    }

    fun onScreenResumed() {
        repository.resumeSubscriptions()
    }

    override fun onCleared() {
        super.onCleared()
        repository.unsubscribe("state", "_global")
    }
}

// TorrentDetailViewModel.kt

class TorrentDetailViewModel(...) : ViewModel() {

    init {
        // Subscribe to detail data for this torrent
        // Each tab subscribes to its own data type
    }

    fun onTabSelected(tab: DetailTab) {
        // Unsubscribe from previous tab, subscribe to new tab
        // In practice, each view subscribes to a single type
        when (tab) {
            DetailTab.PEERS -> {
                repository.unsubscribeAll(infoHash)
                repository.subscribe("peers", infoHash, intervalMs = 1000)
            }
            DetailTab.FILES -> {
                repository.unsubscribeAll(infoHash)
                repository.subscribe("files", infoHash, intervalMs = 2000)
            }
            DetailTab.TRACKERS -> {
                repository.unsubscribeAll(infoHash)
                repository.subscribe("trackers", infoHash, intervalMs = 5000)
            }
        }
    }

    fun onScreenPaused() {
        repository.pauseSubscriptions()
    }

    fun onScreenResumed() {
        repository.resumeSubscriptions()
    }

    override fun onCleared() {
        super.onCleared()
        repository.unsubscribeAll(infoHash)
    }
}
```

### Payload Structure

Combined payload sent on each push:

```typescript
interface StatePayload {
  // Included when subscribed to 'state' (hash: '_global')
  torrents?: TorrentSummary[]

  // Included based on per-torrent subscriptions
  peers?: Record<string, PeerInfo[]>
  files?: Record<string, FileInfo[]>
  trackers?: Record<string, TrackerInfo[]>
  pieces?: Record<string, PiecesData>  // includes piece map, changes, active states
  details?: Record<string, TorrentDetails>
}

interface PiecesData {
  bitfield: string        // base64 or hex encoded
  recentChanges: number[] // piece indices completed since last push
  activeStates: string    // compact encoding of in-progress pieces
}
```

### Kotlin Payload Parsing

```kotlin
// EngineController.kt - parse combined payload

fun handleStateUpdate(json: String) {
    val payload = Json.parseToJsonElement(json).jsonObject

    // Parse torrent list (present only if subscribed to 'state' type)
    val torrents = payload["torrents"]?.let { parseTorrents(it) }

    // Parse per-torrent subscription data
    val peers = payload["peers"]?.let { parsePeers(it) }
    val files = payload["files"]?.let { parseFiles(it) }
    val trackers = payload["trackers"]?.let { parseTrackers(it) }
    val pieces = payload["pieces"]?.let { parsePieces(it) }  // includes bitfield, changes, active
    val details = payload["details"]?.let { parseDetails(it) }

    // Update state - only update fields that are present in payload
    _state.update { current ->
        current.copy(
            torrents = torrents ?: current.torrents,
            peers = peers ?: current.peers,
            files = files ?: current.files,
            trackers = trackers ?: current.trackers,
            pieces = pieces ?: current.pieces,
            details = details ?: current.details
        )
    }
}
```

## Implementation Chunks

Split into 3 chunks for manageable agent context:

### Chunk 1: JS SubscriptionManager

**Scope:** `packages/engine/` only

- [ ] Create `src/adapters/native/subscriptions.ts` with SubscriptionManager class
- [ ] Add global function bindings (`__jstorrent_subscribe`, `__jstorrent_unsubscribe`, etc.)
- [ ] Wire into engine initialization (create manager, pass to native adapter)
- [ ] Update `StatePayload` type to include optional subscription fields
- [ ] Move/refactor `buildPayload` logic from existing push code
- [ ] Unit tests for SubscriptionManager

**Boundary:** No Kotlin changes. Existing push loop still works. New bindings exist but aren't called yet.

### Chunk 2: Kotlin Integration

**Scope:** `android/` only

- [ ] Add subscription methods to EngineController (`subscribe`, `unsubscribe`, `pauseSubscriptions`, etc.)
- [ ] Extend `EngineState` with peers/files/trackers/details fields
- [ ] Update `handleStateUpdate` to parse new payload fields
- [ ] Update TorrentListViewModel to subscribe to 'state' type
- [ ] Update TorrentDetailViewModel to call subscribe/unsubscribe for detail types
- [ ] Add pause/resume tied to screen lifecycle
- [ ] Remove RPC fetch calls from ViewModels

**Boundary:** JS RPC handlers still exist (unused). ViewModels now use subscriptions.

### Chunk 3: Cleanup

**Scope:** Both `packages/engine/` and `android/`

- [ ] Remove RPC handlers from JS: `getPeers`, `getFiles`, `getTrackers`, etc.
- [ ] Remove corresponding Kotlin RPC methods
- [ ] Remove old polling code remnants
- [ ] Delete unused types

### Future: Load Adaptation

- [ ] Measure tick drift in engine to detect overload
- [ ] Slow push frequency when behind
- [ ] Consider skipping non-essential data when severely overloaded

## Testing

### Unit tests (JS)
- Subscribe adds to subscription set
- Unsubscribe removes from subscription set
- UnsubscribeAll clears all for a hash
- Pause stops the push loop
- Resume restarts the push loop
- Subscribe restarts loop immediately (fast first update)
- Most recent subscribe sets the interval
- Torrent removal cleans up subscriptions
- Payload includes all subscribed data
- Torrent list only included when 'state' type subscribed for '_global'
- Piece data (bitfield, changes, active) included in 'pieces' subscription
- Empty payload when no subscriptions

### Unit tests (Kotlin)
- Subscribe/unsubscribe calls reach engine
- Pause/resume calls reach engine
- Payload parsing handles all field types
- Nullable fields handled correctly (data present only when subscribed)

### Integration tests
- End-to-end subscription flow
- Rapid tab switching (subscriptions update correctly)
- App backgrounding calls pause, foregrounding calls resume
- Torrent removal while viewing detail screen
- ViewModel cleanup on navigation

## Resolved Design Questions

1. **Immediate first push** - `subscribe()` restarts the push loop immediately, so first update arrives without waiting for interval.

2. **Subscription lifecycle** - No TTL. Use `pause()`/`resume()` for screen visibility. Auto-cleanup when torrent removed.

3. **Push frequency** - Most recent `subscribe()` call sets the interval for all subscriptions. In practice, each view subscribes to a single type, so this is straightforward.

4. **Single subscriber** - One UI per engine, no subscriber IDs needed. Future remote UI would get its own manager.

5. **Base state is opt-in** - The torrent list is only pushed when subscribed to 'state' type with hash '_global'. Piece data (bitfield, changes, active states) is per-torrent via the 'pieces' subscription.

6. **Use Set for subscription types** - `Map<string, Set<SubscriptionType>>` for O(1) add/remove instead of array indexOf/splice.

## Future Work

1. **Load adaptation** - Engine could measure tick drift to detect overload, then slow push frequency automatically.

2. **Remote UI** - If/when needed, register separate subscription managers per connection ID.
