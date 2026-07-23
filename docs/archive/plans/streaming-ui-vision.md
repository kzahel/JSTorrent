# Streaming UI — Vision, Current Status, and Remaining Work

See also:
- [on-demand-streaming.md](on-demand-streaming.md) — technical architecture (Source interface, abort/cancellation, keyframe index extraction, segment flow)
- [streaming-e2e-plan.md](streaming-e2e-plan.md) — tactical implementation plan (TorrentSource blocking reads, Node E2E test, MP4-first approach)

## Vision

JSTorrent streaming has two faces:

### 1. Power User (embedded player)

Click a video file in the Files tab → full-screen overlay covers the torrent UI, pauses the render loop. Close button returns to normal view. Same page, same JS context, engine calls are direct function calls.

### 2. Casual User (standalone web page)

Open `jstorrent.com/watch#xt=urn:btih:HASH&dn=Movie.mkv` → video player, nothing else. No torrent UI, no file lists, no peers. The page connects to the local daemon, adds the torrent, streams, and cleans up on tab close. The user never sees the word "torrent." Share button generates the same hash-fragment URL — magnet params stay client-side, never hit the server.

This is the primary marketing surface: "paste a magnet link and watch instantly."

## Status Snapshot

As of March 10, 2026, much of the original Phase 1 work is already in place, but the doc below was written before the implementation settled.

### Implemented

- **Embedded player in the main client UI**: clicking a video file can open the in-app React player overlay (`packages/client/src/AppContent.tsx`, `packages/client/src/components/VideoPlayer.tsx`).
- **Extension popup player**: the Chrome extension can launch a dedicated popup player window, backed by a `BroadcastChannel` proxy to the shared playback session (`packages/client/src/utils/video-popup-session.ts`, `packages/client/src/components/VideoPopupPage.tsx`, `extension/src/sw.ts`).
- **Android standalone/native playback**: Android has a native `PlayerActivity` using Media3/ExoPlayer and a torrent-backed byte source (`android/app/src/main/java/com/jstorrent/app/player/PlayerActivity.kt`, `android/app/src/main/java/com/jstorrent/app/player/TorrentPlaybackDataSource.kt`).
- **Shared byte-session + playback-controller split**: the engine already exposes `ByteRangeStreamingSession`, `StreamingPlayerController`, playback capabilities/options, prepared metadata, and diagnostics (`packages/engine/src/streaming/streaming-file-provider.ts`, `packages/engine/src/streaming/streaming-playback-session.ts`).
- **Playback mode selection**: current controlled players can choose between `direct-bytes` and `hls`, with the React player defaulting through `playsvideo` and optionally preferring a daemon-minted direct URL when available.

### Not Implemented Yet

- Public watch page (`jstorrent.com/watch`)
- Daemon HTTP streaming endpoints described below
- Share flow for watch URLs
- `jstorrent://watch` custom protocol flow
- Ephemeral watch-page torrent lifecycle / cleanup-on-tab-close
- Separate `packages/player/` workspace package
- Render-loop / polling pause for the main UI while video overlay is open

### Discovery & Launch

The watch page needs to find the local daemon. This varies by browser:

- **Chrome**: Cross-origin fetch to `localhost:7800` works. Stream directly.
- **Safari**: Cross-origin localhost is blocked. Show "Open in JSTorrent" button → `jstorrent://watch#...` custom protocol link → Tauri app launches daemon → opens `localhost:7800/watch#...` in system browser (same-origin, works everywhere).
- **No daemon found**: Install CTA (extension, desktop app, or Android app depending on platform).

The daemon can also serve the player page itself at `localhost:7800/watch` for offline use and to avoid CORS entirely. `jstorrent.com/watch` is the shareable entry point that redirects there when possible.

### Cleanup Behavior

When the watch page is the entry point (not the extension), the torrent is ephemeral:
- Added on page load, starts downloading immediately
- Removed with data on tab close
- No persistence, no session, no UI for managing it

---

## Library Split: playsvideo + @jstorrent/player

### playsvideo (library, published to npm)

Playsvideo is restructured from a web app into a library + app. The library handles everything video: container parsing, segment planning, hls.js integration, audio transcoding. It accepts a Source (the mediabunny abstraction for byte-level reads) and a `<video>` element.

```typescript
import { PlaysVideoEngine } from 'playsvideo'

const engine = new PlaysVideoEngine(videoElement, source)
await engine.load()    // parse container, build segment plan, start playback
engine.destroy()       // cleanup
```

The Source interface (see [on-demand-streaming.md § Source Interface](on-demand-streaming.md#source-interface) for full details):

```typescript
_read(start, end, signal?: AbortSignal) → ReadResult | Promise<ReadResult> | null
```

- `ReadResult` — data available synchronously
- `Promise<ReadResult>` — data coming (e.g., torrent pieces being downloaded)
- `null` — data cannot be obtained

Playsvideo doesn't know where bytes come from. The Source is the only integration point.

### Abort/Cancellation

Detailed in [on-demand-streaming.md § Abort/Cancellation](on-demand-streaming.md#abortcancellation). Summary:

- `AbortSignal` flows from hls.js fLoader → playsvideo segment processing → Source `_read()`
- TorrentSource listens on the signal to deprioritize pieces and reject pending promises
- Demux/transcode are fast — let them finish, discard results if aborted
- playsvideo owns the signal lifecycle; jstorrent's Source reacts to it

### @jstorrent/player (JSTorrent package)

Thin integration layer. Creates a TorrentSource, passes it to PlaysVideoEngine, owns the torrent-aware loading UI.

```
@jstorrent/player
  ├── playsvideo          (video pipeline — Source in, playback out)
  └── @jstorrent/engine   (Torrent type, waitForPieces, readFileBytes, etc.)
```

**TorrentSource** (`_read`): prioritizes pieces, returns a Promise that resolves when pieces arrive. Listens on AbortSignal to deprioritize and reject on seek. See [on-demand-streaming.md](on-demand-streaming.md#abortcancellation) for the implementation sketch.

**Loading UI**: JSTorrent watches torrent stats directly (peers, speed, piece availability) to show connecting/buffering/ready states. This is torrent-specific UX that doesn't belong in playsvideo.

**Dependency graph:**

```
@jstorrent/player
  ├── @jstorrent/engine   (piece-level primitives)
  └── playsvideo           (video pipeline)

@jstorrent/client
  └── @jstorrent/player    (mounts the overlay)

jstorrent.com/watch (future)
  └── @jstorrent/player    (standalone, no client needed)
```

The engine has no video dependencies. Android never imports the player package (no MSE). The watch page imports the player directly without the full client.

---

## Streaming Surfaces

This section predates the newer implementation. In practice, the codebase now already has the core split this section was aiming for: a shared byte-range session plus a thin playback-controller layer.

### Byte session

Shared across popup playback, future blocking `206`, and future HLS generation:

```ts
read(offset, length, signal?) => Promise<Uint8Array>
waitForRange?(offset, length, signal?) => Promise<void>
close() => void
```

### Playback control and media prep

Used only by players we control:

- choose playback mode
- enumerate concrete playback options for this file/session
- request metadata preparation
- retrieve prepared playback metadata
- coordinate startup UI/progress

The first concrete split here is already effectively:

- `direct-bytes` for complete files with a daemon-minted HTTP stream URL
- `hls` for the existing `playsvideo` pipeline over the shared byte session

### `open(torrentHash, fileIndex, onProgress?) → StreamInfo`

This is still a useful conceptual API for future daemon/HTTP work, but the current in-process implementation is centered on `createStreamingPlaybackSession(...)` returning a session/handle directly instead of a networked `open` RPC.

**Progress callback** (optional, for loading UI):
```typescript
onProgress({
  phase: 'metadata' | 'ready',
  piecesNeeded: number,
  piecesHave: number,
  bytesNeeded: number,
  downloadSpeed: number,   // bytes/sec
  eta: number | null,      // seconds, null if speed is 0
  peers: number,
})
```

UI maps this to:
- `peers === 0` → "No peers available"
- `speed === 0 && peers > 0` → "Connecting..."
- `eta !== null` → "Ready in ~3s" with progress bar
- `phase === 'ready'` → start playback

**Returns:**
```typescript
{ streamId: string, duration: number, playlist: string /* m3u8 */ }
```

### `segment(streamId, segmentIndex, abortSignal?) → Uint8Array`

Still future daemon/API design. The current React player routes HLS/remux behavior through `playsvideo` on top of the shared byte session rather than through an explicit exported `segment(...)` RPC.

Abort signal propagates through the active byte-range read or wait.

### `close(streamId) → void`

Conceptually still correct. In the current implementation this is just `session.close()` / playback-handle disposal rather than a daemon RPC.

### HTTP Mapping (for daemon)

| RPC | HTTP |
|-----|------|
| `open` | `POST /stream/open` |
| `segment` | `GET /stream/{id}/segment/{n}` |
| `close` | `DELETE /stream/{id}` |

Progress delivered via SSE on the open request, final response is the StreamInfo JSON.

---

## Original Phase 1: Embedded Overlay Player

This section mostly landed, but not exactly as originally written.

### What We Build

1. **`packages/player/` package** — not done. The player code currently lives in `packages/client` plus the shared session/controller types in `packages/engine`.

2. **TorrentSource** — done. `packages/engine/src/streaming/torrent-source.ts` adapts the shared byte session into the `playsvideo` `Source` contract.

3. **StreamingSession** — mostly done, with a somewhat different shape. `createStreamingPlaybackSession(...)` / `StreamingPlaybackSession` owns the byte reads, demand windows, file locking, playback capabilities/options, prepared metadata, and diagnostics. The explicit daemon-style `open`/`segment`/`close` RPC surface is still future work.

4. **StreamingRPC interface** — partially done. The shared session types exist, and the popup player wraps them over `BroadcastChannel`, but there is no single formal transport-agnostic `StreamingRPC` abstraction exported as such.

5. **Player overlay component** — done. The main UI mounts `VideoPlayer` as an overlay over the torrent app and tears it down on close.

6. **Render loop pause** — not done. The overlay exists, but the main UI still keeps its normal polling/refresh behavior.

### What Was Skipped / Is Still Pending

- Watch web page (`jstorrent.com/watch`)
- Daemon HTTP streaming endpoints
- Pop-out windows for the extension are no longer skipped; they exist today
- Custom protocol handler (`jstorrent://`)
- Safari/cross-browser discovery
- Share button
- Audio transcoding (ffmpeg.wasm) — still intentionally deferred; current paths favor browser-native codecs or Android's native media stack

### Verification

```bash
pnpm run typecheck && pnpm run test && pnpm run lint
```
