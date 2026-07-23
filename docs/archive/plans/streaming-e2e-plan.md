# Streaming E2E — Tactical Plan

Concrete steps to get video streaming working end-to-end, starting from a Node.js integration test (no browser) and building up to the player UI.

See also:
- [on-demand-streaming.md](on-demand-streaming.md) — architecture, Source interface, keyframe index extraction, segment flow
- [streaming-ui-vision.md](streaming-ui-vision.md) — UI vision, library split, RPC protocol, Phase 1 overlay

## Design Decisions (settled)

**Blocking Source, not probe loop.** TorrentSource `_read()` returns a Promise that waits for pieces instead of returning null. mediabunny has no read timeouts — it awaits indefinitely. This means mediabunny drives the parsing; we just fulfill reads as pieces arrive. No need for a separate probe loop or discovery scanner.

**MP4 works now, MKV needs upstream fix.** MP4 keyframe index building (`getKeyPacket({ metadataOnly: true })` loop) is purely in-memory after moov parsing — zero additional file reads. MKV with `metadataOnly: true` still seeks to every cluster header across the entire file. Fix needed in mediabunny: build MKV keyframe index directly from parsed Cues entries. Tests being added in playsvideo to document both behaviors. **Start with MP4 only; MKV support blocked on mediabunny fix.**

**File size known before streaming starts.** TorrentSource requires parsed torrent metadata (info dict), so `_retrieveSize()` always returns the real file length. No unsized-source edge case.

## Phase 0: Update TorrentSource (blocking reads)

Change `packages/engine/src/streaming/torrent-source.ts`:

```typescript
// Current: returns null for missing pieces
_read(start, end) {
  for (const p of pieces) {
    if (!torrent.hasPiece(p)) return null
  }
  return torrent.readFileBytes(...)
}

// New: waits for missing pieces, supports AbortSignal
_read(start, end, signal?) {
  const pieces = torrent.fileBytesToPieces(fileIndex, start, end - start)
  torrent.setStreamingPieces(new Set(pieces))

  signal?.addEventListener('abort', () => {
    // Deprioritize on abort (e.g., seek)
  })

  return torrent.waitForPieces(pieces, signal)
    .then(() => torrent.readFileBytes(fileIndex, start, end - start))
    .then((bytes) => ({
      bytes,
      view: new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength),
      offset: 0,
    }))
}
```

Note: `setStreamingPieces` replaces the full set each call. During metadata parsing this is fine — reads are sequential and small. During segment loading, the caller (playsvideo/hls.js) manages which segments are in-flight.

### Unit Tests (`test/streaming/torrent-source.test.ts`)

Test the TorrentSource behavior using a mock Torrent (no real downloads, no mediabunny dependency):

1. **Resolves immediately when pieces are available.** Set up a mock torrent where `hasPiece()` returns true for the needed pieces. Call `_read(start, end)`. Assert it resolves with the correct bytes without waiting.

2. **Returns a Promise that resolves when pieces arrive.** Set up a mock torrent where `hasPiece()` initially returns false. Call `_read(start, end)`. Assert it returns a Promise (not resolved yet). Simulate pieces arriving (resolve the `waitForPieces` promise). Assert the `_read` Promise now resolves with correct bytes.

3. **Prioritizes the correct pieces via `setStreamingPieces`.** Call `_read(start, end)`. Assert that `setStreamingPieces` was called with a Set containing exactly the pieces that overlap the requested byte range.

4. **Rejects with AbortError when signal is aborted.** Call `_read(start, end, signal)` where pieces are not yet available. Abort the signal. Assert the Promise rejects with `DOMException` name `'AbortError'`.

5. **Handles reads spanning multiple pieces.** Request a byte range that crosses piece boundaries. Assert `waitForPieces` is called with all overlapping piece indices and the returned bytes are correct.

6. **`_retrieveSize` returns the file length.** Assert it returns `torrent.files[fileIndex].length`.

The mock Torrent needs: `files` array with `length`, `fileBytesToPieces(fileIndex, offset, length)`, `hasPiece(index)`, `waitForPieces(indices, signal)`, `readFileBytes(fileIndex, offset, length)`, and `setStreamingPieces(set)`.

## Phase 1: Node E2E Integration Test

Prove the full pipeline without a browser: seed a video → download via engine → TorrentSource → mediabunny → keyframe index + segment data.

### Setup

- **Video fixture:** Small MP4 file (~50-100KB, a few seconds, H.264+AAC). Can reuse playsvideo's `test-h264-aac.mp4` (53KB) or generate one with ffmpeg.
- **Seeder:** Extend `seed_for_test.py` with a `--file <path>` flag that seeds an existing file instead of generating random data. Or write a dedicated `seed_video_for_test.py`.
- **Test runner:** TypeScript integration test in `packages/engine/integration/` using the Node preset.

### Test Flow

```
1. Python seeder starts, seeds the MP4 fixture via libtorrent
   → outputs magnet link

2. Node engine starts (in-process, Node preset, InMemoryFileSystem or temp dir)
   → adds torrent via magnet link
   → connects to seeder

3. Create TorrentSource(Source, torrent, videoFileIndex)
   → Source base class injected from mediabunny

4. Create mediabunny Input with TorrentSource
   → input.getPrimaryVideoTrack() — blocks until moov is parsed
   → this triggers _read() calls, which wait for pieces from seeder

5. Build keyframe index
   → EncodedPacketSink + getKeyPacket/getNextKeyPacket loop
   → For MP4: zero additional reads (in-memory from moov)

6. Request first segment's packets
   → collectPacketsInRange(videoSink, 0, segmentDuration)
   → blocks on piece downloads for the mdat region

7. Assert:
   - Keyframe index has entries
   - First keyframe timestamp ≈ 0
   - Segment packets have data (byteLength > 0)
   - All pieces for the file were eventually downloaded
```

### Dependencies

The integration test needs mediabunny as a dependency. Options:
- Add `mediabunny` as a devDependency of `@jstorrent/engine` (for integration tests only)
- Or create a separate `packages/player/` package now and put the test there

Prefer adding to engine as devDependency for now — keeps things simple, avoids premature package creation.

### What This Proves

- TorrentSource blocking reads work end-to-end
- mediabunny parses MP4 container from torrent pieces (not a local file)
- Keyframe index extraction works with piece-at-a-time delivery
- Segment packet extraction works
- AbortSignal cancellation can be tested (abort mid-download, verify rejection)

## Phase 2: StreamingSession + Player Package

After E2E test passes:

1. **`packages/player/`** — new workspace package, depends on `playsvideo` and `@jstorrent/engine`
2. **StreamingSession** — implements `open`/`segment`/`close` from the RPC spec
3. **fLoader** — hls.js custom loader that calls `segment()` RPC
4. **Player overlay** — React component with `<video>`, piece canvas, controls

This is the Phase 1 from streaming-ui-vision.md. The E2E test from this plan validates the data pipeline; Phase 2 adds the browser/UI layer.

## Phase 3: MKV Support

Blocked on mediabunny upstream fix. Steps:

1. Submit PR to mediabunny: build MKV keyframe index from Cues entries directly
2. Update playsvideo read-pattern tests to verify fix
3. Add MKV fixture to the E2E test
4. MKV Cues are typically ~20-30KB near end of file — TorrentSource will download first piece (SeekHead) then Cues pieces, same as MP4 moov-at-end

## Open Questions

- **Priority management during segment loading:** Each `_read()` call sets streaming pieces independently. For segment loading (many reads in sequence for one segment), should we pre-compute the full piece set for the segment and set it once? Or is per-read priority fine since reads within a segment are sequential?
- **Multiple in-flight segments:** hls.js may request multiple segments ahead. Each segment's reads go through `_read()` independently. Priority should reflect playback urgency — closest to playhead first. May need a priority queue in TorrentSource.
- **Large moov atoms:** Some MP4s have 5-10MB moov atoms. With 256KB pieces, that's 20-40 pieces. The blocking approach handles this fine (mediabunny reads moov sequentially, each read waits for its piece), but the user sees a longer initial load. Progress reporting should show "parsing metadata" with piece count.
