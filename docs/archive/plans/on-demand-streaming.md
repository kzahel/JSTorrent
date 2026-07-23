# On-Demand Video Streaming

Stream video from an in-progress torrent with instant seek, codec transcoding, and no HTTP server.

See also:
- [streaming-ui-vision.md](streaming-ui-vision.md) — UI vision (overlay vs standalone web page, RPC protocol, package structure, phased rollout)
- [streaming-e2e-plan.md](streaming-e2e-plan.md) — tactical implementation plan (TorrentSource blocking reads, Node E2E test, MP4-first approach)

## Motivation

The legacy implementation (`jstorrent-legacy-app/gui/media.js`, `webhandlers.js`) ran an HTTP server inside the Chrome extension and pointed a `<video>` element at `/stream?hash=X&file=Y`. The browser made Range requests; jstorrent translated those into torrent piece priorities via a "bridge" pattern. This worked but had major limitations:

- **No codec control** — AC3, EAC3, DTS audio just failed with `MEDIA_ERR_NOT_SUPPORTED`
- **No segment awareness** — the browser's video element decided what to buffer and when
- **No keyframe index parsing** — relied on the browser to discover moov/Cues on its own
- **Hacky readiness detection** — counted `progress` events to 40 and assumed playback was working
- **Required an HTTP server** — `web-server-chrome` added complexity and Chrome extension constraints

## Architecture

### Library Split: playsvideo + jstorrent

Playsvideo is restructured from a web app into a **library + app**. The library handles everything video: container parsing, segment planning, hls.js integration, audio transcoding. It accepts a Source (the mediabunny abstraction for byte-level reads) and a `<video>` element.

```typescript
import { PlaysVideoEngine } from 'playsvideo'

const engine = new PlaysVideoEngine(videoElement, source)
await engine.load()    // parse container, build segment plan, start playback
engine.destroy()       // cleanup
```

**playsvideo (library)** owns:
- Container parsing (mediabunny demux)
- Keyframe index extraction
- Segment plan building
- hls.js + fLoader integration
- Audio codec probe + ffmpeg.wasm transcoding
- Abort/cancellation at the segment processing layer

**jstorrent** owns:
- The Source implementation (torrent-piece-backed byte reads)
- Piece prioritization (which pieces to download first)
- Player UI (piece availability canvas, controls)
- Mapping segment byte ranges to torrent pieces

### Source Interface

The Source is the only integration point. It implements mediabunny's read abstraction:

```typescript
_read(start, end, signal?: AbortSignal) → ReadResult | Promise<ReadResult> | null
```

- Returns data if the byte range is available (pieces downloaded).
- Returns a `Promise` if the data is not yet available (pieces being prioritized/downloaded).
- Returns `null` if the data cannot be obtained.

Playsvideo doesn't know where bytes come from — local file, torrent, HTTP, whatever. The Source is the boundary.

### Pipeline

```
torrent pieces → Source → mediabunny (demux) → ffmpeg.wasm (transcode if needed) → fMP4 segments → hls.js (fLoader) → MSE → <video>
```

### Key Components

- **hls.js with `fLoader`** — programmatic segment loading, no HTTP server or service worker needed. hls.js requests segments by index; our loader returns bytes directly from JavaScript.
- **mediabunny** — demuxes MP4 and MKV containers, produces encoded packets for remuxing into fMP4 segments.
- **ffmpeg.wasm (audio-only build, 1.5MB)** — transcodes AC3/EAC3/DTS/FLAC/etc. to AAC on the fly. Lazy-loaded only when the codec probe detects an unsupported audio codec.

### Abort/Cancellation

Uses the standard `AbortController`/`AbortSignal` pattern (same as `fetch()`).

**Flow on seek:**
1. hls.js calls `abort()` on fLoader for in-flight segments
2. playsvideo calls `controller.abort()` — signals fire
3. jstorrent's Source listener deprioritizes those torrent pieces, rejects pending promises
4. playsvideo discards any in-flight demux/transcode results
5. hls.js requests new segments at the seek position
6. playsvideo calls `source._read(start, end, newSignal)` with a fresh signal

**jstorrent Source implementation:**
```typescript
_read(start, end, signal) {
  const pieces = this.piecesForRange(start, end)
  this.prioritize(pieces)

  signal?.addEventListener('abort', () => {
    this.deprioritize(pieces)
  })

  return new Promise((resolve, reject) => {
    signal?.addEventListener('abort', () => {
      reject(new DOMException('Aborted', 'AbortError'))
    })
    this.onPiecesReady(pieces, () => {
      resolve(this.readBytes(start, end))
    })
  })
}
```

**Pipeline abort behavior:**
- **Waiting on Source**: signal fires, promise rejects, pieces deprioritized
- **Demuxing (mediabunny)**: fast/synchronous, let it finish, discard result
- **Transcoding (ffmpeg.wasm)**: can't cancel mid-operation, let it finish, discard result

### Safari / iOS Compatibility

- macOS Safari: MSE supported, hls.js + fLoader works.
- iOS 17+: MSE supported, same as desktop.
- iOS < 17: No MSE. Would require a service worker fallback (not planned — low priority).

## Keyframe Index Extraction

The first step in streaming is parsing the container's keyframe index so we can map seek positions to byte ranges and build a segment plan.

### Strategy: Always Download the First Torrent Piece

The first piece of the file is sufficient to bootstrap index discovery for all common formats.

### By Container Format

**MP4** (`.mp4`, `.m4v`, `.mov`):
- Keyframe index lives in the `moov` atom (`stss` sync samples, `stco`/`co64` chunk offsets, `stsz` sample sizes).
- `moov` is at the beginning (fast-start) or end of the file.
- From the first piece, chain top-level box headers (`offset += size`) to locate `moov`. If `moov` is at the end, the `mdat` box's size tells you exactly where `moov` starts — pure arithmetic, no scanning.
- Edge case: `mdat` with `size=0` means "rest of file", implying `moov` must be before `mdat` (already in first piece).

**MKV / WebM** (`.mkv`, `.webm`):
- Keyframe index is the `Cues` element (maps timestamps → byte offsets of clusters).
- `Cues` are typically at the end of the file.
- The `SeekHead` element is always near the start (~first 1-4KB). It contains the exact byte offset and size of `Cues`.
- Parse SeekHead from first piece → compute which pieces contain Cues → prioritize those (usually 1 piece, Cues are ~20-30KB for a 2-hour movie).

**WebM is a subset of MKV** — same EBML container, same SeekHead/Cues structure, same code path.

### Moov/Cues Size Heuristics

| Format | Index element | Typical size (2hr movie) | Driven by |
|--------|--------------|-------------------------|-----------|
| MP4 | `moov` | 2-10 MB | Total frame count × tracks |
| MKV | `Cues` | 20-30 KB | Keyframe count only |

MP4 moov is larger because it indexes every frame (`stsz`), not just keyframes. MKV Cues only index keyframes (~one every 5 seconds).

### mediabunny Integration

mediabunny's `Reader` abstraction uses `requestSlice(start, length)` — it fetches data on demand, never reads the whole file. It returns `null` gracefully on missing data rather than throwing. This means:

- A custom `Source` backed by torrent piece availability works naturally.
- For MP4, mediabunny parses moov without ever touching mdat.
- For MKV, it can parse SeekHead and Cues from sparse byte ranges.
- Zero-filled gaps between downloaded regions cause the box parser to stop gracefully (`readBoxHeader` returns null, loop breaks).

## Segment Request Flow

1. Build segment list from keyframe index (Cues/moov → "segment N = keyframe at time T, byte range X-Y").
2. Generate in-memory HLS manifest with segment durations.
3. hls.js requests segments in playback order via `fLoader`.
4. On segment request:
   - Map segment to byte range → compute overlapping torrent pieces.
   - If pieces are downloaded → demux with mediabunny → transcode audio if needed → return fMP4 bytes.
   - If pieces are not downloaded → boost priority of those pieces in the torrent engine → resolve promise when they arrive.
5. On seek: hls.js calls `abort()` on in-flight loads → deprioritize those torrent pieces.

### Priority

Priority is implicit in hls.js call order. The oldest unresolved `fLoader` request is the most urgent (playhead needs it now). Subsequent requests are buffer-ahead. Aborted requests are no longer needed.

### Buffering

hls.js manages its own buffer-ahead target (~30s by default). It decides how far to prefetch. The torrent engine just needs to respond to piece priority changes.

## Formats Supported

For practical purposes, only two container formats matter for torrents:

| Format | Extensions | Prevalence |
|--------|-----------|------------|
| MKV | `.mkv`, `.webm` | ~90% of scene releases, fansubs |
| MP4 | `.mp4`, `.m4v`, `.mov` | Web-sourced, remuxes |

AVI (`.avi`) and FLV (`.flv`) are effectively dead for new content. TS (`.ts`) has no index at all (pure streaming format). None of these are planned.

## Implementation Steps

See [streaming-e2e-plan.md](streaming-e2e-plan.md) for the detailed tactical plan.

1. **Blocking TorrentSource** — `_read()` returns a Promise that prioritizes pieces and waits for download (not null). mediabunny has no read timeouts and drives the parsing. Already exists at `packages/engine/src/streaming/torrent-source.ts`, needs update from null-returning to blocking.
2. **Node E2E test** — seed a video file, download via engine, pipe through TorrentSource → mediabunny → keyframe index → segment data. Proves the pipeline without a browser.
3. **Segment plan builder** — convert keyframe index to HLS segment list with durations.
4. **fLoader implementation** — bridge between hls.js segment requests and torrent piece priorities.
5. **Codec probe + transcode** — reuse playsvideo's `audioNeedsTranscode` and ffmpeg.wasm audio pipeline for AC3/EAC3/DTS.
6. **Player UI** — piece availability visualization (like the old green canvas bar), playhead, seek controls.

**MP4 first, MKV blocked.** MP4 keyframe index building is purely in-memory after moov parsing. MKV `getKeyPacket({ metadataOnly: true })` seeks to every cluster header — needs upstream mediabunny fix to read from Cues directly.
