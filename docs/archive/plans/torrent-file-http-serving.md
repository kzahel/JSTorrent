# Torrent File HTTP Serving

See also:
- [on-demand-streaming.md](on-demand-streaming.md) - current JS `Source`-based streaming pipeline
- [streaming-ui-vision.md](streaming-ui-vision.md) - current player/RPC vision
- [android-native-streaming-player-mvp.md](android-native-streaming-player-mvp.md) - Android standalone native playback path
- [../design_docs/stream-file.md](../design_docs/stream-file.md) - older `/stream` endpoint design
- [../design_docs/io-daemon-design.md](../design_docs/io-daemon-design.md) - older daemon streaming notes

## Purpose

Record the three HTTP serving modes we have discussed for torrent-backed file playback, how they relate to the current implementations, and what would need to change in the ChromeOS extension + Android companion architecture to support them cleanly.

This document is deliberately architecture-focused. It is not a detailed implementation spec for any one endpoint.

## The Three HTTP Modes

### 1. Raw `206` passthrough for complete files

This is the simplest HTTP surface.

- Only exposed when the target file is fully complete on disk.
- Standard browser/media-client `Range` semantics.
- No torrent-aware blocking.
- No special piece-priority behavior.
- Best fast path for direct-play formats that the browser or cast target already supports.

Operationally this is just:

`HTTP Range request -> file stat/validation -> direct disk read -> 206 response`

This does not need the torrent engine once the completeness check has passed.

### 2. Blocking torrent-aware `206`

This is the direct-play path for incomplete files.

- Client sends a normal `Range` request.
- Server maps the requested file byte range to torrent pieces.
- Server raises streaming demand for those pieces.
- Server blocks until the needed pieces are complete.
- Server then reads from disk and returns a normal `206`.
- On disconnect or seek-away, the server cancels the wait and clears demand.

Operationally this is:

`HTTP Range request -> torrent-aware wait/priority -> disk read -> 206 response`

This gives the efficiency and compatibility benefits of a normal media URL while still supporting in-progress torrents.

### 3. HLS over HTTP

This is the most flexible HTTP surface.

- Expose a real `.m3u8` playlist plus segment endpoints.
- Back it with the existing JS media pipeline:
  `Source -> mediabunny -> remux/transcode if needed -> fMP4 segments`
- Supports remuxing/transcoding and broad receiver compatibility.
- Best fit for Chromecast and other remote playback targets that prefer HLS.

Operationally this is:

`HTTP playlist/segment request -> segment pipeline -> HLS response`

This is not as cheap as raw direct-play `206`, but it covers more formats and gives a general network playback surface.

## Recommended Product Shape

These should be treated as three externally visible serving modes over shared internals, not three separate streaming engines.

Shared internals should stay split roughly like this:

- Byte-range session:
  - `read(offset, length)`
  - optional `waitForRange(offset, length)`
  - `close()`
  - internal file byte range -> piece mapping
  - internal wait/cancel/priority behavior
- Playback-control/media-prep service:
  - player-controlled capability discovery
  - per-file playback option discovery
  - metadata preparation
  - prepared playback metadata retrieval
- Segment service:
  - byte source -> demux/remux/transcode -> HLS segments
- HTTP adapters:
  - complete-file `206`
  - blocking torrent-aware `206`
  - HLS playlist/segment endpoints

Pragmatic rollout order:

1. Complete-file `206`
2. HTTP HLS
3. Blocking torrent-aware `206`

This gets a cheap fast path first, then a flexible cast/network path, then the more involved torrent-aware direct-play path.

For controlled players, the first concrete direct-play option should be a daemon-minted HTTP stream URL for complete files. That lets the player controller choose between:

- `direct-bytes` when the file is complete and the browser can probably direct-play it
- `hls` otherwise

## Implementation Matrix

### Legacy Chrome extension app

The legacy app already had a version of mode 2.

- It ran an HTTP server inside the extension.
- The browser `<video>` pointed directly at `/stream?hash=...&file=...`.
- The browser made normal `Range` requests.
- The handler translated those range requests into torrent bridge/prioritization behavior.

Relevant files:

- [archive/legacy-app/js/webhandlers.js](/Users/kgraehl/code/jstorrent/archive/legacy-app/js/webhandlers.js)
- [archive/legacy-app/js/torrent.js](/Users/kgraehl/code/jstorrent/archive/legacy-app/js/torrent.js)

Characteristics:

- Served real HTTP `206` responses.
- Understood enough torrent semantics to block until requested regions became available.
- Was efficient for direct-play browser formats.
- Did not support the current remux/transcode/HLS pipeline.

### Current extension embedded/popup player

The current player does not expose a public HTTP media endpoint.

- `VideoPlayer` creates a `PlaysVideoEngine` with a torrent-backed `Source`.
- `StreamingPlaybackSession.read()` handles byte-range-to-piece mapping, `waitForPieces`, streaming demand, and cancellation.
- HLS exists inside the JS player pipeline via `playsvideo`/`hls.js`, not as public HTTP endpoints.
- The popup/player may request media prep because it is a controlled client, but that should remain separate from the shared byte-session contract.
- The next controller step is to let the player choose a complete-file daemon URL fast path before falling back to that HLS pipeline.

Relevant files:

- [packages/client/src/components/VideoPlayer.tsx](/Users/kgraehl/code/jstorrent/packages/client/src/components/VideoPlayer.tsx)
- [packages/engine/src/streaming/torrent-source.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/streaming/torrent-source.ts)
- [packages/engine/src/streaming/streaming-playback-session.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/streaming/streaming-playback-session.ts)

Characteristics:

- Most of the torrent-aware logic already exists here.
- No public `206` endpoint today.
- No public HLS manifest/segment endpoint today.
- Best current reference for where torrent semantics should remain.

### ChromeOS extension + Android companion (`io-daemon`-like)

This is the main architecture constraint for new HTTP serving.

Current shape:

- Extension/JS engine owns torrent semantics.
- Android companion owns local disk/filesystem serving plus control and socket surfaces.
- File reads from daemon-backed storage are custom HTTP random-access reads:
  `/read/{rootKey}` plus `X-Path-Base64`, `X-Offset`, `X-Length`.
- The extension drives daemon requests.
- The daemon does not currently initiate engine RPCs.

Relevant files:

- [packages/engine/src/adapters/daemon/daemon-file-handle.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/daemon/daemon-file-handle.ts)
- [packages/engine/src/adapters/daemon/daemon-connection.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/daemon/daemon-connection.ts)
- [packages/engine/src/adapters/daemon/control-connection.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/daemon/control-connection.ts)
- [android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt](/Users/kgraehl/code/jstorrent/android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt)
- [android/companion-server/src/main/java/com/jstorrent/companion/server/ControlWebSocketHandler.kt](/Users/kgraehl/code/jstorrent/android/companion-server/src/main/java/com/jstorrent/companion/server/ControlWebSocketHandler.kt)

Mode 1 in this architecture:

- Straightforward.
- Check whether the file is complete.
- If complete, serve a standard `Range` endpoint directly from disk.
- The torrent engine only needs to participate in gating exposure of the endpoint.

Mode 2 in this architecture:

- This is the tricky one.
- The daemon would need a way to ask the extension/engine:
  - whether a byte range is readable yet
  - to wait for it
  - to raise or cancel streaming demand
- Media bytes would still come from disk/HTTP, not WebSocket.
- The control plane would carry only small wait/ready/cancel messages.
- Token registration must be owned by the same `/control` session that answers
  those wait/close requests. Registering the token from some other client
  session defeats the ownership model and makes lifecycle cleanup ambiguous.

Mode 3 in this architecture:

- Also reasonable.
- The extension/engine can remain the "brain" that owns torrent semantics and segment generation.
- HTTP HLS can be exposed either:
  - from an extension-side HTTP surface, or
  - from the companion with a control-plane bridge back to the engine.

### Android standalone app

Android standalone is intentionally different.

- It uses Media3 `DataSource`, not HTTP `Range`.
- It is explicitly not HLS in the MVP.
- It opens playback sessions directly against the JS engine and blocks on reads there.
- If Android standalone also exposes tokenized `/stream/{token}`, that should
  reuse the same HTTP surface as companion mode but swap in a local app-owned
  wait backend instead of the extension-owned `/control` backend.

Relevant files:

- [android/app/src/main/java/com/jstorrent/app/player/TorrentPlaybackDataSource.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/player/TorrentPlaybackDataSource.kt)
- [android/app/src/main/java/com/jstorrent/app/player/EnginePlaybackByteSource.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/player/EnginePlaybackByteSource.kt)
- [docs/plans/android-native-streaming-player-mvp.md](/Users/kgraehl/code/jstorrent/docs/plans/android-native-streaming-player-mvp.md)

Characteristics:

- Not an HTTP-serving architecture.
- Useful reference for the blocking byte-read model.
- Not the main target for the HTTP serving work described here.

## Where Torrent Semantics Live Today

Today the torrent-aware behavior lives on the engine side, not the daemon side.

The engine already owns:

- file byte range -> piece mapping
- piece completion / bitfield state
- `waitForPieces`
- streaming demand / file locking
- abort/cancellation behavior
- media-prep implementation details such as sparse metadata reads for keyframe/index building

This logic is concentrated in:

- [packages/engine/src/streaming/streaming-playback-session.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/streaming/streaming-playback-session.ts)
- [packages/engine/src/core/torrent.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/core/torrent.ts)
- [packages/engine/src/core/torrent-content-storage.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/core/torrent-content-storage.ts)

The daemon is comparatively thin:

- stateless file reads/writes
- control broadcasts
- socket multiplexing
- authentication

That thinness is desirable and should generally be preserved.

## Architectural Guidance

### Complete-file `206`

Keep this entirely daemon-side once the extension has determined the file is complete enough to expose.

This should remain thin and mostly independent of torrent internals.

### Blocking torrent-aware `206`

Prefer to keep torrent semantics in the engine and use the daemon as:

- HTTP request terminator
- disk reader
- waiter/canceller on behalf of HTTP clients

In other words:

- daemon parses `Range`
- daemon asks engine to wait for a byte range
- engine decides when ready
- daemon serves bytes from disk

This avoids duplicating torrent logic inside Rust/Kotlin daemon code.

The daemon-facing contract should stay byte-oriented. It should not grow a public hint API just because the engine uses internal prioritization.

### HLS over HTTP

Prefer to keep segment logic close to the current JS streaming stack:

- `Source`
- `StreamingPlaybackSession`
- `playsvideo`
- demux/remux/transcode

Expose that over HTTP instead of rebuilding the media pipeline elsewhere.

HLS should reuse the same byte-range session as `206`, with additional media-prep and segment-planning services layered above it.

## Minimum Control Protocol Extension for ChromeOS Companion

The current `/control` WebSocket is the natural place to add daemon <-> extension coordination for mode 2 and potentially mode 3 metadata/session control.

Do not use `/io` for this.

Reasons:

- `/io` is currently for sockets/file-write protocol traffic.
- `/control` already has request/response framing with `requestId` plus JSON payloads.
- Media bytes do not need to move over WebSocket.

Relevant current files:

- [android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt](/Users/kgraehl/code/jstorrent/android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt)
- [android/companion-server/src/main/java/com/jstorrent/companion/server/ControlWebSocketHandler.kt](/Users/kgraehl/code/jstorrent/android/companion-server/src/main/java/com/jstorrent/companion/server/ControlWebSocketHandler.kt)
- [extension/src/lib/daemon-bridge/chromeos/ws-connect.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/daemon-bridge/chromeos/ws-connect.ts)
- [extension/src/lib/daemon-bridge/chromeos/ws-requests.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/daemon-bridge/chromeos/ws-requests.ts)
- [extension/src/lib/daemon-bridge/protocol/control-frame.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/daemon-bridge/protocol/control-frame.ts)

### Current protocol shape

Today `/control` is effectively:

- authenticated binary envelope
- JSON payloads
- `requestId` correlation
- extension-initiated requests
- daemon-initiated broadcasts/events

What it does not currently model is daemon-initiated RPC that expects the extension to do work and send a correlated response.

That is the real protocol change required for blocking torrent-aware `206`.

### Proposed additions

The new pattern should be treated as bidirectional RPC over the existing `/control` frame format.

That means:

- either side may send a request frame with a nonzero `requestId`
- the peer may respond on the same opcode or on a dedicated response opcode
- the daemon may also send a cancellation frame tied to the original request token

This does not require a new frame format.
It does require:

- new control opcodes
- daemon-side pending-request tracking
- extension-side inbound request dispatch
- explicit cancellation handling

### Suggested control opcodes

These names are conceptual. Final numeric opcode assignment can follow the existing control-plane pattern.

1. `OP_CTRL_REGISTER_HTTP_STREAM`

- Extension -> daemon
- Registers a streamable file/session
- Payload could include:
  - `streamToken`
  - `torrentId`
  - `fileIndex`
  - `rootKey`
  - `path`
  - `fileSize`
  - `mimeType`

2. `OP_CTRL_OPEN_HTTP_STREAM_SESSION`

- Daemon -> extension
- Opens request-scoped playback state for one HTTP reader
- Payload:
  - `sessionId`
  - `streamToken`
  - `torrentId`
  - `fileIndex`

3. `OP_CTRL_WAIT_FOR_HTTP_STREAM_RANGE`

- Daemon -> extension
- Asks engine to wait for and prioritize a byte range
- Payload:
  - `sessionId`
  - `streamToken`
  - `torrentId`
  - `fileIndex`
  - `offset`
  - `length`

4. `OP_CTRL_CANCEL_HTTP_STREAM_RANGE_WAIT`

- Daemon -> extension
- Cancels a pending wait because the HTTP client disconnected, timed out, or sought away
- Payload:
  - `sessionId`
  - `reason`

5. `OP_CTRL_CLOSE_HTTP_STREAM_SESSION`

- Either direction
- Closes session state and clears file locks / streaming demand
- Payload:
  - `sessionId`
  - `reason`

6. `OP_CTRL_REVOKE_TORRENT_HTTP_STREAMS`

- Extension -> daemon
- Proactively revokes all registered stream tokens for a removed torrent
- Payload:
  - `torrentId`
  - `reason`

7. `OP_CTRL_RANGE_WAIT_RESULT`

- Extension -> daemon
- Completes a prior `WAIT_FOR_RANGE` request
- Payload:
  - `sessionId`
  - `ok`
  - `error?`
  - `status?`

This can also be modeled as "respond on the same opcode and correlate by `requestId`", but an explicit result opcode is easier to reason about and debug.

### Suggested payloads

`REGISTER_STREAM_SESSION`

```json
{
  "streamId": "s_123",
  "infoHash": "abc123...",
  "fileIndex": 4,
  "rootKey": "downloads",
  "path": "Movies/example.mp4",
  "fileSize": 734003200,
  "mimeType": "video/mp4"
}
```

`WAIT_FOR_RANGE`

```json
{
  "streamId": "s_123",
  "waitToken": "w_456",
  "offset": 1048576,
  "length": 262144
}
```

`RANGE_WAIT_RESULT`

```json
{
  "streamId": "s_123",
  "waitToken": "w_456",
  "ok": true
}
```

Failure case:

```json
{
  "streamId": "s_123",
  "waitToken": "w_456",
  "ok": false,
  "status": 503,
  "error": "torrent stopped"
}
```

`CANCEL_RANGE_WAIT`

```json
{
  "streamId": "s_123",
  "waitToken": "w_456"
}
```

`CLOSE_STREAM_SESSION`

```json
{
  "streamId": "s_123"
}
```

### Directionality

The protocol needs to support both of these patterns at once:

- extension -> daemon RPC
  - already exists today for KV and open-file/open-folder operations
- daemon -> extension RPC
  - needed for `WAIT_FOR_RANGE`

That means the extension must gain a new inbound request dispatcher for control opcodes, not just event and response handlers.

### Required implementation changes

At minimum:

1. Add the new control opcodes to [Protocol.kt](/Users/kgraehl/code/jstorrent/android/io-core/src/main/java/com/jstorrent/io/protocol/Protocol.kt) and include them in `CONTROL_OPCODES`.
2. Update route-validation tests in [WebSocketRouteTest.kt](/Users/kgraehl/code/jstorrent/android/app/src/test/java/com/jstorrent/app/server/WebSocketRouteTest.kt).
3. Extend [ControlWebSocketHandler.kt](/Users/kgraehl/code/jstorrent/android/companion-server/src/main/java/com/jstorrent/companion/server/ControlWebSocketHandler.kt) with:
   - stream-session registry
   - pending wait registry
   - request sending for `WAIT_FOR_RANGE`
   - cancel handling
4. Extend [ws-connect.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/daemon-bridge/chromeos/ws-connect.ts) to dispatch daemon-initiated control requests, not only broadcasts and known response opcodes.
5. Add extension-side request handlers that call the existing engine-side primitives:
   - `fileBytesToPieces`
   - `updateStreamingDemand`
   - `waitForPieces`
   - cleanup on cancel

### Why this is still lightweight

The new protocol adds control complexity, not data-plane complexity.

It does not require:

- media bytes over WebSocket
- high-throughput frame handling
- daemon-owned torrent metadata
- daemon-owned piece state

It only requires a small RPC/control surface so the daemon can ask the engine:

- "is this range readable yet?"
- "wake me when it is"
- "cancel that wait"

That is enough for blocking `206`. Media-prep actions for controlled players or HLS planning should be modeled separately from these byte-wait operations.

### Why this is enough

This keeps the companion protocol small.

The daemon does not need:

- torrent metadata
- piece maps
- bitfield subscriptions
- media bytes over WebSocket
- public hint semantics

It only needs:

- a way to ask "wake me when this range is readable"
- a way to cancel that wait
- a way to clean up session state

## How Mode 2 Would Work on ChromeOS Companion

High-level flow:

1. Extension registers a stream session with daemon.
2. Remote client requests `GET /stream/{streamId}` with `Range`.
3. Daemon parses the range.
4. Daemon sends `WAIT_FOR_RANGE(streamId, waitToken, offset, length)` over `/control`.
5. Extension:
   - maps bytes to pieces
   - raises streaming demand
   - waits until readable
   - responds success or failure
6. Daemon reads the bytes from disk using the already existing file-read path.
7. Daemon returns standard HTTP `206`.
8. If the HTTP request aborts, daemon sends `CANCEL_RANGE_WAIT`.

Important point:

The WebSocket is not a data plane here. It is only a control plane.

## How HLS Over HTTP Could Work

There are two viable shapes:

### Shape A: extension-side segment service

- The extension owns segment generation and torrent semantics.
- HTTP endpoints proxy to engine-generated playlist/segment results.
- Lowest duplication of current JS streaming logic.

### Shape B: daemon-side HTTP shell with extension-owned brain

- The daemon exposes `/stream/open`, `/stream/{id}/playlist.m3u8`, `/stream/{id}/segment/{n}`.
- The daemon asks the extension for:
  - open/session metadata
  - playlist text
  - segment bytes or segment readiness

In either shape, the HLS layer should sit above the same byte-range session used for blocking `206`. The difference is in media prep and segment orchestration, not in the underlying byte contract.
- Still keeps torrent semantics in JS, but pushes more orchestration into the daemon.

Shape A is closer to the current implementation.
Shape B is closer to a LAN-visible service model for Chromecast and external clients.

## Capability Matrix

| Mode | Complete files | Incomplete files | Direct-play efficiency | Remux/transcode capable | Good fit for Chromecast |
| --- | --- | --- | --- | --- | --- |
| Complete-file `206` | Yes | No | Best | No | Limited to direct-play formats |
| Blocking torrent-aware `206` | Yes | Yes | Very good | No | Good for direct-play casts |
| HLS over HTTP | Yes | Yes | Lower than direct-play | Yes | Best compatibility |

## Open Questions

1. Should the first public HTTP fast path be complete-file `206` only, with no incomplete-file behavior?
2. Should HLS endpoints be implemented before blocking torrent-aware `206` because they are more broadly useful for Chromecast?
3. Should LAN-visible HTTP serving live in the companion/daemon, or should the extension own the public HTTP surface and use the daemon only for disk reads?
4. How should ephemeral auth/session tokens be attached to public media URLs?
5. For mode 2, how aggressively should the daemon cancel pending waits when the HTTP client changes ranges frequently?

## Current Recommendation

Near-term:

- Implement complete-file `206` as the cheap fast path.
- Implement HTTP HLS as the compatibility path.

Later:

- Add blocking torrent-aware `206` using `/control` WebSocket request/response extensions, keeping torrent semantics on the engine side and disk reads on the daemon side.

That preserves the current separation of responsibilities while still enabling:

- efficient direct-play playback
- flexible HLS playback
- eventual torrent-aware direct-play over normal HTTP
