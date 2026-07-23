# Torrent Queue System Implementation Plan

## Overview

Implement a torrent queue system inspired by libtorrent's auto-management. The engine limits how many torrents can be simultaneously active (downloading/seeding). Excess torrents are set to `queued` state and automatically promoted when active slots free up.

**Key simplifications vs libtorrent:**
- No `dont_count_slow_torrents` (future candidate)
- No `seed_rank` scoring — queue position is the sole ordering for all states
- No separate DHT/tracker/LSD announce limits
- No `auto_manage_prefer_seeds`

## Existing Scaffolding

These pieces already exist and will be built upon:

- `TorrentUserState` already includes `'queued'` (`torrent-state.ts:4`)
- `Torrent.queuePosition` getter/setter exists, persisted via `session-persistence.ts`
- `computeActivityState()` treats `'queued'` as `'stopped'` (`torrent-state.ts:37`) — will need change
- Kill switch in `Torrent` blocks networking for `userState === 'queued'` (`torrent.ts:1408`)
- Android `StatusBadge` already maps `"queued"` to "Queued" label with `secondary` color
- Android `TorrentSummary` already has `userState` field that includes `"queued"`
- Extension `TorrentSummary` already includes `userState` field
- `ConfigHub` reactive settings system with schema, validation, and UI binding

## User State Model

Following libtorrent's `auto_managed` concept, we add a `forceActive` flag:

| userState | forceActive | Meaning |
|-----------|-------------|---------|
| `'active'` | false | Queue-managed: queue decides if running or queued |
| `'active'` | true | Force-started: bypasses queue limits, always runs |
| `'stopped'` | false | User-paused: skipped by queue entirely |
| `'queued'` | false | Set by queue manager when active torrent is over limit |

**Play/pause button behavior:**
- **Queued torrent**: shows "pause" button (user's intent is active, pressing pause → stopped). Status displays "Queued"
- **Stopped torrent**: shows "play" button (pressing play → `userState = 'active'`, enters queue)
- **Force-active torrent**: shows "pause" button as normal

**Long-press / "..." menu actions:**
- "Force Start" — sets `forceActive = true`, starts immediately regardless of limits
- "Move to Top of Queue" — moves queue position to 0
- "Move to Bottom of Queue" — moves queue position to last

---

## Phase 1: Engine Queue Manager (Core Logic)

**Goal:** Add a `TorrentQueueManager` class to the engine that enforces active torrent limits and auto-promotes queued torrents.

### Config Settings (`config-schema.ts`)

Add to the schema under a new "Queue" section:

| Key | Type | Default | Min/Max | Storage | Exposed in UI |
|-----|------|---------|---------|---------|--------------|
| `activeDownloads` | number | 5 | 1 / 50 | sync | Yes |
| `activeSeeds` | number | 5 | 1 / 50 | sync | Yes |

Hardcoded constants (not settings):
- `ACTIVE_CHECKING = 1` — max concurrent hash-checking torrents
- `ACTIVE_LIMIT = 500` — hard cap on total active auto-managed torrents
- `AUTO_MANAGE_INTERVAL_MS = 5000` — periodic re-evaluation fallback

### Torrent Persisted State Changes

Add to `TorrentPersistedState` (`torrent.ts:~85`):
```typescript
forceActive?: boolean  // default false, bypasses queue limits
```

Add to `TorrentStateData` (`session-persistence.ts:~65`):
```typescript
forceActive?: boolean
```

### New File: `packages/engine/src/core/torrent-queue-manager.ts`

```
class TorrentQueueManager extends EngineComponent {
  constructor(engine: BtEngine, config: ConfigHub)

  // Called by engine when queue needs re-evaluation
  recalculate(): void

  // Queue position manipulation
  moveToTop(torrent: Torrent): void
  moveToBottom(torrent: Torrent): void

  // Force start (bypass queue)
  forceStart(torrent: Torrent): void

  // Called by engine on lifecycle events
  onTorrentAdded(torrent: Torrent): void
  onTorrentRemoved(torrent: Torrent): void
  onTorrentCompleted(torrent: Torrent): void

  // Assign initial queue positions to torrents that don't have them (upgrade path)
  assignInitialPositions(torrents: Torrent[]): void
}
```

**`recalculate()` algorithm:**
1. Collect torrents by category:
   - `forceActive` torrents: always stay active, not counted against limits
   - `userState === 'stopped'` torrents: skipped entirely
   - Downloading torrents (`progress < 1, userState !== 'stopped'`): sorted by `queuePosition` asc
   - Seeding torrents (`progress >= 1, userState !== 'stopped'`): sorted by `queuePosition` asc
2. For downloading queue: first `activeDownloads` non-force torrents → ensure `userState = 'active'` and `start()`. Remainder → set `userState = 'queued'`, call `stopNetwork()` if was active
3. For seeding queue: first `activeSeeds` non-force torrents → same logic
4. Persist state changes
5. Log transitions (e.g., "Torrent X: active → queued (position 6, limit 5)")

**Queue position management:**
- Single shared position space across all torrents (download + seed)
- Contiguous integers starting from 0
- New torrents appended to end: `queuePosition = max(all positions) + 1`
- `moveToTop()`: sets to 0, shifts others +1
- `moveToBottom()`: sets to max, shifts others -1 to close gap
- On removal: shifts positions to close gap
- On completion (download → seed): keeps same position, just reclassified

**Trigger points for `recalculate()`:**
- `onTorrentAdded()` — after assigning queue position
- `onTorrentRemoved()` — after closing position gap
- `onTorrentCompleted()` — frees download slot
- `userStart()` / `userStop()` — user intent changed
- `activeDownloads` / `activeSeeds` config changes (via subscription)
- Engine `resume()` after suspend
- Session restore complete (single batch call)
- Periodic: every 5 seconds via engine tick (fallback, catches edge cases)

**Debounce:** Queue recalculation is debounced — if called multiple times within a tick, only runs once (at end of current tick via microtask or flag).

### Activity State Change (`torrent-state.ts`)

Update `computeActivityState()` to return `'queued'` when `userState === 'queued'`:

```typescript
// Currently: if (userState === 'stopped' || userState === 'queued') return 'stopped'
// Change to:
if (userState === 'stopped') return 'stopped'
if (userState === 'queued') return 'queued'
```

Add `'queued'` to `TorrentActivityState` type:
```typescript
export type TorrentActivityState =
  | 'stopped' | 'checking' | 'downloading_metadata'
  | 'downloading' | 'seeding' | 'error'
  | 'queued'  // NEW: waiting for active slot
```

### Integration with `BtEngine` (`bt-engine.ts`)

- Create `TorrentQueueManager` in constructor (after config is available)
- In `addTorrent()`: call `queueManager.onTorrentAdded(torrent)` instead of unconditionally calling `torrent.start()`. The queue manager decides whether to start or queue.
- In `removeTorrent()`: call `queueManager.onTorrentRemoved(torrent)` before removing
- On `'complete'` event handler: call `queueManager.onTorrentCompleted(torrent)`
- In `resume()`: call `queueManager.recalculate()` instead of manually iterating and starting
- Wire config subscriptions for `activeDownloads` / `activeSeeds`
- Add public methods:
  - `queueMoveToTop(torrent: Torrent)`
  - `queueMoveToBottom(torrent: Torrent)`
  - `queueForceStart(torrent: Torrent)`
- Modify `Torrent.userStart()` path: after setting `userState = 'active'`, call `queueManager.recalculate()` instead of unconditionally starting
- Modify `Torrent.userStop()` path: after stopping, call `queueManager.recalculate()` to promote next queued

### Native Controller (`controller.ts`)

Add global command functions:
- `__jstorrent_cmd_queue_top(infoHash)` → `engine.queueMoveToTop()`
- `__jstorrent_cmd_queue_bottom(infoHash)` → `engine.queueMoveToBottom()`
- `__jstorrent_cmd_force_start(infoHash)` → `engine.queueForceStart()`

### Subscription Data (`subscriptions.ts`)

Add `queuePosition` to `TorrentSummary` interface and `buildTorrentSummary()`:
```typescript
queuePosition: number | undefined  // undefined if not assigned
forceActive: boolean
```

### Files Modified
- `packages/engine/src/config/config-schema.ts` — add `activeDownloads`, `activeSeeds`
- `packages/engine/src/config/config-hub.ts` — add accessor properties
- `packages/engine/src/config/memory-config-hub.ts` — add accessor properties
- `packages/engine/src/core/torrent-state.ts` — add `'queued'` to `TorrentActivityState`, return `'queued'` from `computeActivityState()`
- `packages/engine/src/core/torrent.ts` — add `forceActive` to persisted state, adjust `userStart()` / `userStop()` to trigger queue recalc
- `packages/engine/src/core/bt-engine.ts` — integrate queue manager, add public queue APIs, change `addTorrent()` and `resume()` flow
- `packages/engine/src/core/session-persistence.ts` — persist/restore `forceActive`
- `packages/engine/src/adapters/native/controller.ts` — add queue command handlers
- `packages/engine/src/adapters/native/subscriptions.ts` — add `queuePosition`, `forceActive` to `TorrentSummary`

### New Files
- `packages/engine/src/core/torrent-queue-manager.ts`

### Tests
- `packages/engine/test/core/torrent-queue-manager.test.ts`
  - Adding N+1 torrents (where N = activeDownloads) queues the last one
  - Removing an active torrent promotes the next queued one
  - Completing a download frees a download slot, promotes next queued downloader
  - `moveToTop()` / `moveToBottom()` reorder correctly and trigger recalculate
  - User-stopped torrents are skipped by the queue
  - Starting a stopped torrent puts it at its queue position; may or may not run depending on limits
  - `forceStart()` bypasses queue limits
  - Settings changes (activeDownloads/activeSeeds) trigger recalculation
  - Queue positions stay contiguous after removal
  - Upgrade path: torrents without queuePosition get assigned by addedAt order

### Verification
- `pnpm run typecheck`
- `pnpm run test` (new + existing tests pass)
- `pnpm run lint`
- `pnpm format:fix`

---

## Phase 2: Extension UI — Queue Column and Actions

**Goal:** Add queue position column to torrent table, add "Move to Top/Bottom" + "Force Start" to context menu, add active download/seed limit settings.

### Torrent Table Column (`packages/ui/src/tables/TorrentTable.tsx`)

Add a `queue` column:
- ID: `queue`
- Header: `#`
- Width: 40px
- Align: right
- Value: `t.queuePosition !== undefined ? t.queuePosition + 1 : '-'` (1-based display)
- Sortable, not hidden by default
- Position: first column (before `name`)

### Status Column Enhancement

When `activityState === 'queued'`, display "Queued" with a distinct muted style. Add a cell style case in the status column renderer, e.g. `color: 'var(--text-secondary)'`.

### Context Menu (`packages/client/src/AppContent.tsx`)

Add after the "Stop" item, before the separator:
```
{ id: 'forceStart', label: 'Force Start', icon: '⏩', disabled: allForceActive }
{ id: 'separator-queue', separator: true }
{ id: 'moveToTop', label: 'Move to Top of Queue', icon: '⤒' }
{ id: 'moveToBottom', label: 'Move to Bottom of Queue', icon: '⤓' }
```

Wire `handleMenuAction` cases for these actions.

### EngineAdapter Extension (`packages/client/src/adapters/types.ts`)

Add to `EngineAdapter` interface:
```typescript
queueMoveToTop(torrent: Torrent): void
queueMoveToBottom(torrent: Torrent): void
queueForceStart(torrent: Torrent): void
```

Implement in `DirectEngineAdapter` by delegating to `engine.queueMoveToTop()` etc.

### Settings UI (`packages/client/src/components/SettingsOverlay.tsx`)

In the **Network** tab, add a "Queue" section (after "Connection Limits"):
- **Max Active Downloads**: number input (min 1, max 50, default 5)
- **Max Active Seeds**: number input (min 1, max 50, default 5)

Wire to config keys `activeDownloads` and `activeSeeds`.

### Files Modified
- `packages/ui/src/tables/TorrentTable.tsx` — add queue column, queued status style
- `packages/client/src/AppContent.tsx` — add context menu items + handlers
- `packages/client/src/adapters/types.ts` — add queue methods to interface + implementation
- `packages/client/src/components/SettingsOverlay.tsx` — add Queue settings section

### Verification
- `pnpm run typecheck`
- `pnpm run test`
- `pnpm run lint`
- `pnpm format:fix`
- Manual: open extension, verify queue column appears, right-click menu shows queue actions, settings show queue limits

---

## Phase 3: Android UI — Queued State and Actions

**Goal:** Show queued state in Android torrent list, add "Move to Top/Bottom" + "Force Start" actions, add QUEUED filter tab, add queue settings.

### TorrentSummary Update (`EngineModels.kt`)

Add to `TorrentSummary`:
```kotlin
val queuePosition: Int? = null,
val forceActive: Boolean = false,
```

### Filter Tabs (`TorrentListScreen.kt`)

Change tabs from `ALL | ACTIVE | FINISHED` to `ALL | ACTIVE | QUEUED | FINISHED`:
- **QUEUED**: filters to `status == "queued"`
- **ACTIVE**: remains `status in ["downloading", "downloading_metadata", "checking"]`
- Update `TorrentFilter` enum in `UiState.kt` and `filterByStatus()` extension

### TorrentCard Behavior

- When `status == "queued"`: play/pause button shows **pause** icon (user intent is active; tapping pauses → stopped)
- StatusBadge already handles "queued" → "Queued" in secondary color (no change needed)

### Torrent Detail Overflow Menu (`TorrentDetailScreen.kt`)

Add queue actions to the overflow ("...") menu:
- "Force Start" — calls `viewModel.forceStart()`
- "Move to Top of Queue"
- "Move to Bottom of Queue"

### Selection Action Bar

Consider adding "Force Start" to `SelectionActionBar.kt` alongside Play/Pause/Delete, or keep it only in the detail menu for simplicity. Recommendation: detail menu only (keeps the selection bar clean).

### Engine Controller Bridge (`EngineController.kt`)

Add:
```kotlin
suspend fun queueMoveToTopAsync(infoHash: String) {
    requireEngine().callGlobalFunctionAsync("__jstorrent_cmd_queue_top", infoHash)
}
suspend fun queueMoveToBottomAsync(infoHash: String) {
    requireEngine().callGlobalFunctionAsync("__jstorrent_cmd_queue_bottom", infoHash)
}
suspend fun forceStartAsync(infoHash: String) {
    requireEngine().callGlobalFunctionAsync("__jstorrent_cmd_force_start", infoHash)
}
```

### Repository & ViewModel

- `TorrentRepository.kt`: add `queueMoveToTop(infoHash)`, `queueMoveToBottom(infoHash)`, `forceStart(infoHash)`
- `EngineServiceRepository.kt`: implement via `withEngine { ... }`
- `TorrentListViewModel.kt`: expose these actions
- `TorrentDetailViewModel.kt`: expose these actions

### Settings Screen (`SpeedConnectionLimitsSettingsScreen.kt`)

Add "Queue" section:
- **Max Active Downloads**: stepper/slider (1-50, default 5)
- **Max Active Seeds**: stepper/slider (1-50, default 5)

Wire through `SettingsViewModel` → engine config.

### Files Modified
- `android/quickjs-engine/.../EngineModels.kt` — add `queuePosition`, `forceActive`
- `android/quickjs-engine/.../EngineController.kt` — add queue commands
- `android/app/.../viewmodel/TorrentRepository.kt` — add queue methods
- `android/app/.../viewmodel/EngineServiceRepository.kt` — implement queue methods
- `android/app/.../viewmodel/TorrentListViewModel.kt` — add queue actions, update filter
- `android/app/.../viewmodel/TorrentDetailViewModel.kt` — add queue actions
- `android/app/.../model/UiState.kt` — update `TorrentFilter` enum
- `android/app/.../ui/screens/TorrentListScreen.kt` — add QUEUED tab
- `android/app/.../ui/screens/TorrentDetailScreen.kt` — add queue menu items
- `android/app/.../ui/components/TorrentCard.kt` — queued state play/pause behavior
- `android/app/.../ui/screens/SpeedConnectionLimitsSettingsScreen.kt` — add queue settings section
- `android/app/.../viewmodel/SettingsViewModel.kt` — add queue settings
- `android/app/.../cache/TorrentSummaryCache.kt` — update if needed for new fields

### Verification
- `./gradlew :app:compileDebugKotlin`
- `./gradlew testDebugUnitTest`
- Manual: add >5 torrents on emulator, verify queue behavior, QUEUED tab, overflow menu actions

---

## Phase 4: Graceful Transitions & Upgrade Path

**Goal:** Ensure graceful stop when queueing active torrents, smooth upgrade for existing users, robust session restore.

### Graceful Stop on Queue Deactivation

When `recalculate()` decides to move a torrent from active → queued:
- Don't abruptly disconnect peers — allow in-flight piece requests to complete
- Add `Torrent.gracefulStop(timeoutMs = 10000)`:
  1. Stop requesting new pieces (don't call `requestPieces()` in tick)
  2. Set a flag `_gracefulStopping = true`
  3. After timeout or all in-flight responses received, call `stopNetwork()`
  4. Set `userState = 'queued'`
- The queue manager uses `gracefulStop()` instead of immediate `stopNetwork()` when deactivating

### Session Restore

- On restore, apply persisted `queuePosition` and `forceActive` values
- After ALL torrents are restored (end of `restoreSession()`), call `queueManager.recalculate()` once
- `addTorrent()` with `source: 'restore'` should NOT trigger individual recalculates — use a batch flag

### Upgrade Path (First Run with Queue System)

When existing users upgrade, their torrents have `queuePosition = undefined`:
- `assignInitialPositions()` runs on first `recalculate()` if any torrents lack positions
- Assigns positions based on `addedAt` timestamp (oldest = 0, newest = highest)
- Active downloading torrents beyond `activeDownloads` limit get queued
- This is a visible behavior change — document in changelog

### User Override Semantics

Clear behavioral rules:
- **User clicks "Start" on stopped torrent**: sets `userState = 'active'`, `forceActive = false`. Queue manager decides: if under limit, starts; if over, queues.
- **User clicks "Stop" on active torrent**: sets `userState = 'stopped'`. Frees slot; `recalculate()` promotes next queued.
- **User clicks "Stop" on queued torrent**: sets `userState = 'stopped'`. Removed from queue consideration.
- **User clicks "Force Start"**: sets `userState = 'active'`, `forceActive = true`. Starts immediately, does NOT count against limits.
- **Torrent completes download**: keeps position, reclassified from download to seed queue. Frees download slot; `recalculate()` promotes next queued downloader.
- **Torrent errors**: remains in its position but network stops. Queue manager skips errored torrents.

### Files Modified
- `packages/engine/src/core/torrent.ts` — add `gracefulStop()`, `_gracefulStopping` flag
- `packages/engine/src/core/torrent-queue-manager.ts` — use graceful stop, batch restore mode
- `packages/engine/src/core/session-persistence.ts` — batch restore support
- `packages/engine/src/core/bt-engine.ts` — coordinate restore with queue manager

### Tests
- Add to `torrent-queue-manager.test.ts`:
  - Force-start bypasses limits, doesn't displace others
  - User stop frees slot and promotes next
  - Session restore respects persisted queue positions
  - Upgrade path: unpositioned torrents get assigned by addedAt
  - Graceful stop: torrent finishes in-flight before stopping (mock)

### Verification
- `pnpm run typecheck && pnpm run test && pnpm run lint && pnpm format:fix`
- Manual: add 8 torrents, verify 5 download and 3 queued. Stop one active, verify promotion. Kill/restart, verify state preserved.

---

## Design Decisions

### Single queue position space
Both downloaders and seeds share the same position sequence. When a torrent finishes, it keeps its position and simply reclassifies. Simpler than maintaining two separate queues.

### `forceActive` flag vs separate userState
Using a boolean flag alongside `userState` is cleaner than adding a fourth user state. Force-active torrents are still `userState = 'active'` — the flag just tells the queue manager to skip them when counting against limits.

### No `dont_count_slow_torrents` yet
Adds complexity (rate tracking, debounce, hysteresis). The basic queue provides immediate value. Can layer on later.

### Seed queue ordering
Uses queue position (not libtorrent's availability-based `seed_rank`). Simple and predictable. Seeding queue behavior isn't worth over-indexing on.

### Upgrade behavior for existing users
All existing torrents start as `userState = 'active'` with no queue position. On first recalculate: positions assigned by addedAt, excess torrents queued. This is a visible change — must be in changelog.
