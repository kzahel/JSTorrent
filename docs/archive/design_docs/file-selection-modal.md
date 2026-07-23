# File Selection Modal

## Overview

When a user adds a torrent (magnet or .torrent file), optionally show a modal that lets them choose which files to download and where to save them before any file data is transferred. This doubles as an onboarding improvement: the modal requires a download location to be set before proceeding, eliminating the current error state when no storage root is configured.

## Motivation

- Users with large torrents often only want specific files
- Current flow lets users start torrents without a download location configured, leading to confusing error states
- Other clients (qBittorrent, Transmission) provide this as a standard feature

## User-Facing Behavior

### Setting

"Show file selection when adding torrents" — stored in user preferences.

Options:
- **Always** (default for new users) — show for every torrent added
- **Never** — current behavior, add and start immediately

### Modal Layout

Top to bottom:

1. **Torrent name** — from magnet `dn=` parameter, .torrent file name, or info hash
2. **Download location** — dropdown of configured storage roots. Each shows free disk space. If none configured, shows a prompt to add one. **Download button disabled until a location is selected.**
3. **File tree** — hierarchical with checkboxes per file and folder-level aggregation. Shows file sizes. If metadata not yet available (magnet), shows a spinner in this area.
4. **Summary bar** — "X files selected, Y GB" vs "Z GB free on selected location". Warning color if selected > free.
5. **Actions:**
   - **Download** — set file priorities from selection, set download location, transition to active. Disabled until location is set (and at least one file selected, if metadata available).
   - **Download All** — skip file selection, download everything to selected location. Available even while metadata is loading. For magnets where metadata hasn't arrived, torrent proceeds and downloads all files once metadata arrives.
   - **Cancel** — remove the torrent entirely.

### "Don't show again" checkbox

In the modal. Flips the global setting to "Never".

### Multiple Torrents

One modal rendered at a time. The queue is derived from all torrents in `awaitingFileSelection` state, ordered by `addedAt` timestamp. Dismissing/confirming the current modal reveals the next one (if any). Queued torrents are visible in the main torrent list with a distinct visual state.

### .torrent Files vs Magnets

Same flow for both. The only difference is whether the file tree is available immediately (.torrent — metadata parsed locally, essentially instant) or after a wait (magnet — metadata fetched from peers via BEP 9).

## Engine Changes

### New `TorrentUserState`: `'awaitingFileSelection'`

Add `'awaitingFileSelection'` to `TorrentUserState` in `core/torrent-state.ts`. No new persisted fields — this is just a new value for the existing `userState`, which is already persisted in the session store.

Semantics when `userState === 'awaitingFileSelection'`:
- Torrent is active in the network (connects to peers, fetches metadata via BEP 9)
- All pieces are treated as unwanted — no piece data is requested, regardless of `filePriorities`
- No storage root needs to be assigned yet (metadata is stored in the session KV store, not on disk)
- Distinct from "user skipped all files" — this is a pre-download state where the user hasn't made a choice yet

### New `TorrentActivityState`: `'awaitingFileSelection'`

Add `'awaitingFileSelection'` to `TorrentActivityState`. Update `computeActivityState`:
- `userState === 'awaitingFileSelection'` and `!hasMetadata` → `'downloading_metadata'`
- `userState === 'awaitingFileSelection'` and `hasMetadata` → `'awaitingFileSelection'`

This lets the UI distinguish "fetching metadata from peers" from "metadata ready, waiting for user to pick files."

### Piece Selection Gate

The piece selection logic must check `userState`. When `userState === 'awaitingFileSelection'`, all pieces are unwanted — the torrent does not request any piece data from peers. This prevents the current problem where a torrent with no storage root assigned would start downloading and error on write.

### Adding Torrents

The UI controls whether to use this state. When the user's "show file selection" preference is enabled, the UI passes `userState: 'awaitingFileSelection'` to `addTorrent()`. The existing `addTorrent(input, { userState })` option already supports this — no new parameter needed.

When the preference is disabled, the UI passes `userState: 'active'` (or omits it for the default) and the torrent starts immediately as today.

### Confirming File Selection (UI-driven)

No new engine methods required. The UI composes existing operations:

1. **"Download" (specific files):** Set file priorities via `torrent.setFilePriority()` for each file, assign storage root, set `torrent.userState = 'active'`.
2. **"Download All":** Assign storage root, set `torrent.userState = 'active'`. File priorities stay at default (normal) — downloads everything. Works even before metadata arrives: once metadata arrives, all files are normal priority and downloading starts.
3. **"Cancel":** Remove the torrent via `engine.removeTorrent()`.

### Metadata Event

When metadata is received for a torrent with `userState === 'awaitingFileSelection'`, the engine emits an event (e.g., `'metadata-ready'`) so the UI can populate the file tree. The engine does NOT auto-start piece downloads — the piece selection gate handles this.

### Persistence / Restart

On app restart, torrents restore with their persisted `userState`. A torrent with `userState: 'awaitingFileSelection'` restores into that state — it reconnects to peers but doesn't download pieces. The UI can show these torrents in the list with a distinct visual state; whether to re-show the modal is a UI decision.

## Client / Adapter Changes

Minimal. The adapter already exposes `addTorrent()` with `userState`, file priority methods, and storage root assignment. The UI composes these for the confirm flow. The adapter needs to expose:

- File list and metadata status for awaiting torrents (for populating the file tree)
- Storage roots (already exposed)

No new adapter methods needed — `confirmFileSelection` / `confirmAllFiles` / `cancelAwaitingTorrent` are UI-level compositions of existing engine operations.

## Free Disk Space

`IFileSystem.getFreeDiskSpace()` is implemented across all backends. Each `IFileSystem` instance is scoped to one storage root, so the method takes no parameters and returns available bytes (or -1 if unsupported). Backend capability negotiated via `free_space` boolean in `StatusCapabilities` / `DaemonCapabilities` for backward/forward compat — old backends missing the field are treated as unsupported, and the UI hides free space gracefully.

## Platform Notes

### Extension / Tauri

Modal is part of the extension UI (React). All engine calls are direct method calls on the JS engine instance — no new daemon/companion endpoints needed since the engine runs in the same JS context (extension UI page or Tauri webview).

### Android (standalone mode)

Android has its own native Compose UI. The file selection flow needs a native equivalent — a Compose screen/dialog with the same behavior. Engine changes are shared since the same TypeScript engine runs in QuickJS.

### iOS

Same as Android — native SwiftUI equivalent needed. Engine changes shared via JavaScriptCore.

### Node CLI

Not applicable for modal UI. CLI users would use the `so=` magnet parameter or a future `--select-files` flag.

## Implementation Plan

### Phase 1: Engine state model + tests ✅

**State changes (`core/torrent-state.ts`):**
- Add `'awaitingFileSelection'` to `TorrentUserState`
- Add `'awaitingFileSelection'` to `TorrentActivityState`
- Update `computeActivityState` to handle the new user state

**Piece selection gate:**
- Where the engine decides which pieces to request, check `userState` — when `'awaitingFileSelection'`, report all pieces as unwanted

**Tests (using `createMemoryEngine`, `InMemoryFileSystem`, `TorrentCreator`, memory socket pairs):**

Unit tests for `computeActivityState`:
- `awaitingFileSelection` + no metadata → `'downloading_metadata'`
- `awaitingFileSelection` + has metadata → `'awaitingFileSelection'`
- After setting `userState = 'active'` → normal activity states resume

Integration tests with memory engine:
- Add torrent with `userState: 'awaitingFileSelection'` from .torrent buffer — verify torrent is created, has metadata, activity state is `'awaitingFileSelection'`, no pieces requested
- Add magnet with `userState: 'awaitingFileSelection'` — verify activity state is `'downloading_metadata'`
- Two-client memory swarm: seeder has data, leecher adds magnet in `awaitingFileSelection` — verify leecher receives metadata (activity state transitions to `'awaitingFileSelection'`) but downloads zero pieces
- Confirm with specific files: set file priorities, assign root, set `userState = 'active'` — verify only selected files' pieces are requested, downloading starts
- Confirm all (before metadata): assign root, set `userState = 'active'` — verify that once metadata arrives, all pieces are downloaded
- Cancel: remove torrent — verify cleanup
- Persistence: add torrent in `awaitingFileSelection`, save session, restore — verify torrent restores into same state, no pieces requested

### Phase 2: UI (extension/Tauri) ✅

- User preference: "Show file selection when adding torrents" (Always / Never)
- Modal component: torrent name, storage root dropdown, file tree (with spinner while awaiting metadata), summary bar, action buttons
- File tree: hierarchical checkboxes, folder-level aggregation, file sizes
- Queue: one modal at a time, ordered by `addedAt`, next modal appears on dismiss/confirm
- Torrent list: distinct visual state for `awaitingFileSelection` torrents
- "Don't show again" checkbox in modal

### Phase 3: Free disk space ✅

`IFileSystem.getFreeDiskSpace()` implemented across all backends (Node, ScopedNode, Daemon, Native, Memory, Null) and all backend runtimes (Rust io-daemon, Android FileManager/companion, iOS, Node daemon). Capability negotiated via `free_space` flag. Surfaced in modal's storage root dropdown and summary bar warning.

### Phase 4: Android / iOS native UI

Compose dialog (Android) and SwiftUI sheet (iOS) equivalents of the modal. Engine changes from Phase 1 are shared. These are independent of each other and can be done in parallel.
