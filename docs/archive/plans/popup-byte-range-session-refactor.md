# Popup Byte-Range Session Refactor

See also:
- [torrent-file-http-serving.md](torrent-file-http-serving.md) - HTTP serving modes and control-plane direction
- [on-demand-streaming.md](on-demand-streaming.md) - current JS streaming pipeline context
- [streaming-ui-vision.md](streaming-ui-vision.md) - player/watch UX direction
- [android-native-streaming-player-mvp.md](android-native-streaming-player-mvp.md) - Android-native byte-source direction

## Purpose

Record the cleaner long-term boundary for popup playback after the initial byte-range-session refactor landed in code.

The key clarification is that we do not want one generic session surface that mixes:

- byte reads
- torrent prioritization hints
- media-prep orchestration
- diagnostics

Those concerns should be split explicitly.

## Direction

We want two primary surfaces and one optional auxiliary surface.

### 1. Byte session

This is the common substrate for:

- popup playback
- blocking torrent-aware `206`
- HLS segment generation

It should stay minimal and identical across those consumers.

```ts
interface ByteRangeStreamingSession {
  readonly fileSize: number
  read(offset: number, length: number, signal?: AbortSignal): Promise<Uint8Array>
  waitForRange?(offset: number, length: number, signal?: AbortSignal): Promise<void>
  close(): void
}
```

Notes:

- `read()` is the core operation.
- `waitForRange()` is optional but useful for daemon/control-plane parity.
- No public hint API belongs here.
- No piece-aware API belongs here.

### 2. Playback control and media prep

This surface is only for players we control, such as the popup player.

It can be richer because it is not the shared substrate for arbitrary HTTP clients or remote receivers.

Responsibilities here include:

- deciding playback mode based on platform and player capabilities
- exposing concrete playback options for this file/session
- being passive or active about startup
- requesting media metadata preparation when useful
- retrieving prepared playback metadata

Conceptually:

```ts
interface PlaybackControlService {
  getPlaybackCapabilities(): Promise<PlaybackCapabilities>
  getPlaybackOptions(): Promise<PlaybackOption[]>
  preparePlaybackMetadata(kind: PlaybackMetadataKind): Promise<void>
  getPreparedPlaybackMetadata(): Promise<PreparedPlaybackMetadata | null>
}
```

This is the right home for operations like prebuilding a keyframe index. It is not the right home for torrent scheduling hints.

The first concrete non-HLS option should be:

- `direct-bytes` backed by a daemon-minted HTTP stream URL
- only when the file is fully complete on disk
- only on hosts that can register and expose that URL

### 3. Optional diagnostics

Diagnostics stay separate from both of the above.

```ts
interface StreamingVisualization {
  getPieceTimelineSnapshot?(): Promise<StreamingFilePieceSnapshot | null>
}
```

This keeps torrent-aware visualization available without leaking torrent internals into the operational API.

## Why This Split Is Cleaner

### Public clients do not speak hints

We want the future network-facing contracts to be:

- plain HTTP `206` range requests
- HLS playlist and segment requests

Those clients will never send extra torrent-prioritization hints. That means the shared byte-level contract should not require or advertise them.

### Popup does control playback strategy

The popup player is different from an uncontrolled HTTP client.

It may need to choose between:

- direct byte-stream playback
- HLS-style playback
- future mode-specific optimizations

That means the popup legitimately needs playback-control and media-prep APIs. It does not mean it should control torrent demand tokens or piece windows directly.

### Metadata prep is different from byte reads

Operations like `buildPrebuiltKeyframeIndex()` are not byte-session primitives.

They are media-prep operations:

- inspect container metadata
- fetch sparse metadata ranges as needed
- build a reusable playback artifact

Those operations belong on the playback-control/media-prep surface.

## Current Problems

The initial popup refactor improved the transport substantially, but the conceptual boundary still needs cleanup.

Remaining issues:

- the popup/session shape still suggests hints belong on the public session contract
- `buildPrebuiltKeyframeIndex()` still reads like a session RPC rather than a media-prep capability
- diagnostics and operational methods are still too easy to conflate

The next refactor should fix the conceptual boundary, not just rename types.

## Target Popup Model

The popup should receive or construct a handle with clearly separated surfaces:

```ts
interface PopupPlaybackHandle {
  bytes: ByteRangeStreamingSession
  control?: PlaybackControlService
  diagnostics?: StreamingVisualization
}
```

That gives the popup player what it actually needs:

- a uniform byte source
- optional controlled-player APIs for metadata prep and concrete mode selection
- optional torrent diagnostics for UI/debugging

## What Should Stay Internal

These remain engine/internal concerns:

- byte range -> piece mapping
- `waitForPieces`
- streaming demand tokens
- file locks
- forward-download heuristics
- cleanup on abort/seek/close

Even when a controlled player requests metadata preparation, the scheduling mechanics remain internal to the engine.

## Implications For `206` And HLS

### Blocking torrent-aware `206`

The contract is complete with byte reads and waits.

The server can infer urgency from actual request behavior:

- active range reads are highest priority
- aborted reads are canceled
- forward-moving range patterns imply the playback frontier

No public hint surface is required.

### HLS

HLS is not a different byte API.

It is a different consumer/controller over the same byte session.

HLS may need extra media preparation:

- parse container metadata
- build keyframe index
- derive segment plan
- generate init data and segment responses

That work should live in the playback-control/media-prep layer, not in the shared byte session contract.

## Popup Launch Contract

The popup launch descriptor should remain minimal:

- `sessionId`
- `fileName`
- `fileSize`

Transport details like `fileOffset` and `pieceLength` do not belong in the popup launch contract.

## Recommended Refactor Order

### Phase 1: Narrow the core session contract

Goal:

- make the byte session explicitly byte-oriented and remove public hints from its intended long-term contract

Changes:

- keep `read`, `waitForRange`, and `close` as the shared session operations
- treat any torrent prioritization policy as internal engine behavior
- stop using the popup session as the place to define public hint semantics

Expected result:

- popup, `206`, and HLS can all describe the same underlying byte session cleanly

### Phase 2: Introduce a playback-control/media-prep surface

Goal:

- give controlled players a richer API without polluting the byte layer

Changes:

- define a `PlaybackControlService`-style contract for controlled-player operations
- add a concrete `getPlaybackOptions()` method for per-file session options
- move `buildPrebuiltKeyframeIndex()` conceptually into that surface
- make playback capabilities and prepared metadata explicit

Expected result:

- popup can choose between HLS and complete-file direct-byte playback without owning torrent scheduling

### Phase 3: Split popup transport by concern

Goal:

- make the transport shape match the architectural split

Changes:

- proxy byte-session methods separately from playback-control/media-prep methods
- keep diagnostics as an optional auxiliary surface
- remove public hint methods from popup transport

Expected result:

- popup transport no longer suggests that hints are part of the shared session model

### Phase 4: Reuse the same byte contract for daemon work

Goal:

- make future daemon `206` and HLS work reuse the same substrate

Changes:

- document daemon/control-plane operations in terms of byte waits and lifecycle
- keep media-prep operations separate from blocking range waits
- do not invent daemon-specific torrent-hint APIs

Expected result:

- daemon protocols align with the same byte session already used by popup playback

## Success Criteria

- the shared byte session contract is just bytes plus lifecycle
- popup-specific playback-control APIs are clearly separate
- metadata prep is described as a controlled-player/media service, not a byte-session primitive
- no public contract depends on torrent hints or piece-aware operations
- `206` and HLS are described as different consumers over the same byte substrate

## Practical Next Step

The next implementation step should be the popup-side split:

1. Keep the byte session surface minimal.
2. Move `buildPrebuiltKeyframeIndex()` and related media-prep work behind a playback-control surface.
3. Remove public hint methods from the popup-facing contract.

That is the cleanest path toward a future where:

- popup remains flexible and capability-aware
- daemon `206` stays simple
- HLS shares the same byte substrate
- torrent scheduling details stay internal
