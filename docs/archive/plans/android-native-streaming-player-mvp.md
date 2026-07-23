# Android Native Streaming Player MVP

**Goal:** Launch an in-progress video file from the Android standalone app into a dedicated native player activity backed by Media3 and torrent-piece reads.

**Primary user flow:** In the Files tab for a torrent, tap an incomplete video file. The app starts or resumes the torrent, prioritizes that file for streaming, opens a fullscreen `PlayerActivity`, and plays the file natively. If the user backgrounds the app while playback is active, the player can enter Picture-in-Picture (PiP).

## Scope

### In scope

- Native Android playback using Media3/ExoPlayer
- Dedicated `PlayerActivity`
- Launch from existing torrent file list UI
- Streaming from incomplete files
- File-targeted piece prioritization for playback
- PiP while playback is active
- Basic buffering / stalled / unsupported-format UI

### Out of scope

- WebView-based player
- HLS playlist/segment pipeline
- Browser parity with desktop streaming UI
- ffmpeg transcoding
- Kotlin port of MKV cue parsing
- Advanced timeline visualization / keyframe seek UI
- Cross-torrent watch page / share URLs

## Why This Shape

The repo already has the hard torrent-side primitives:

- prepare file for playback (`unskip` + `start torrent`)
- map file byte ranges to pieces
- wait for pieces
- read file bytes
- optional MKV cue parsing as a separate metadata helper

The MVP should preserve those ideas but replace the browser player boundary with Media3:

- Desktop/extension: browser player + JS `Source`
- Android MVP: Media3 + native blocking `DataSource`

This keeps the streaming model familiar while avoiding WebView, MSE, HLS, and codec-transcode work in v1.

## Existing Integration Points

These are the main app surfaces the MVP should plug into:

- Files list UI: [FilesTab.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/tabs/FilesTab.kt)
- Torrent detail screen: [TorrentDetailScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentDetailScreen.kt)
- Navigation host: [Navigation.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/navigation/Navigation.kt)
- App manifest: [AndroidManifest.xml](/Users/kgraehl/code/jstorrent/android/app/src/main/AndroidManifest.xml)

Related existing streaming logic and helpers:

- Desktop/watch playback prep: [watch-video.ts](/Users/kgraehl/code/jstorrent/packages/client/src/utils/watch-video.ts)
- Streaming file provider contract: [streaming-file-provider.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/streaming/streaming-file-provider.ts)
- Optional MKV metadata helper: [mkv-keyframe-index.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/streaming/mkv-keyframe-index.ts)

## Settled Design Decisions

### 1. Progressive native playback first

Do not start with HLS or segment remuxing. Give Media3 a file-backed `DataSource` that blocks on torrent bytes becoming available.

### 2. Dedicated activity, not overlay or WebView

Playback should live in a dedicated `PlayerActivity`, which can feel like a separate app surface and cleanly support PiP.

### 3. File-targeted demand, not whole-torrent streaming mode

When playback starts, the torrent engine should treat the selected file as the active streaming target. This should permanently unskip that file and temporarily bias download demand toward that file's byte range.

### 4. MKV cue parsing is optional metadata, not a blocker

The MVP should not depend on cue parsing. Media3 should be allowed to drive reads directly. Cue parsing can be added later for smarter seek UX and buffering hints.

## Architecture

### Launch flow

1. User taps a video file in the Files tab.
2. UI checks whether the file is a supported video candidate.
3. App ensures the file is unskipped and the torrent is active.
4. App launches `PlayerActivity` with `torrentHash` and `fileIndex`.
5. `PlayerActivity` resolves the torrent/file and constructs a Media3 player.
6. Media3 reads through `TorrentDataSource`.
7. `TorrentDataSource` converts requested byte ranges into torrent piece demand and blocks until bytes are available.

### Core components

#### `PlayerActivity`

Owns:

- Media3 player lifecycle
- fullscreen playback UI
- PiP entry/exit behavior
- user-facing buffering/error state

Inputs:

- `torrentHash`
- `fileIndex`
- maybe `fileName` for immediate title rendering

#### `TorrentPlaybackCoordinator`

Android-side orchestration layer responsible for:

- validating the target torrent/file
- unskipping the file if needed
- ensuring the torrent is started
- applying playback-specific demand / file lock state
- cleaning up when playback ends

This should keep `PlayerActivity` thinner and isolate playback policy.

#### `TorrentDataSource`

Media3 `DataSource` implementation that:

- receives `DataSpec(position, length)`
- maps the requested byte range to torrent pieces
- raises streaming demand for those pieces
- waits for missing pieces
- reads bytes into Media3's buffers
- cancels outstanding demand on `close()`

This is the main technical risk and the heart of the MVP.

## MVP Phases

### Phase 0: Plumbing and Entry Point

### Goal

Make tapping a video file launch a dedicated player activity with stable arguments.

### Work

- Add a lightweight video-file detector on Android
- Update torrent detail file-tap handling to route video files to playback instead of `FileOpener`
- Add `PlayerActivity` to the manifest
- Add a small launcher helper for `torrentHash` + `fileIndex`

### Files expected to change

- [TorrentDetailScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentDetailScreen.kt)
- [FilesTab.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/tabs/FilesTab.kt)
- [AndroidManifest.xml](/Users/kgraehl/code/jstorrent/android/app/src/main/AndroidManifest.xml)

### Verification

- Tap incomplete `.mp4` in Files tab opens `PlayerActivity`
- Tap non-video file still uses existing file opener behavior

### Phase 1: Playback Preparation

### Goal

Mirror the desktop "watch" prep behavior in Android-native code.

### Work

- Add Android-side helper equivalent to desktop `prepareTorrentForVideoPlayback`
- Ensure:
  - skipped file becomes selected for download
  - torrent starts if stopped or errored
  - active streaming target is recorded
- Decide whether playback should also lower priority for sibling files during streaming

### Notes

This policy should not live inside the UI composable. Put it in a playback coordinator or repository/service layer.

### Verification

- Starting playback on a skipped file makes it selected
- Starting playback on a stopped torrent resumes the torrent
- Closing playback does not re-skip the file

### Phase 2: `TorrentDataSource`

### Goal

Feed Media3 directly from torrent-backed byte reads.

### Required behavior

- `open(DataSpec)` records the requested range
- `read()` blocks on missing pieces from a loader thread, not UI thread
- `close()` cancels pending waits and clears demand
- sequential reads do not repeatedly thrash piece priorities
- seek causes old demand to cancel and new demand to replace it

### Internal API shape

The Media3 adapter should wrap a narrower playback-facing byte source, something like:

```kotlin
interface TorrentByteSource {
    suspend fun open(position: Long, length: Long?): Long
    suspend fun read(position: Long, buffer: ByteArray, offset: Int, length: Int): Int
    fun close()
}
```

`TorrentDataSource` then adapts this to Media3's blocking API.

### Open questions within this phase

- How large should the active demand window be beyond the immediate requested range?
- Should we maintain one demand token per open request, per read window, or per player session?
- What is the minimum buffering threshold before we call `playWhenReady = true`?

### Verification

- Progressive MP4 plays from an incomplete file
- Seek forward on an incomplete file reprioritizes pieces and recovers
- Cancel/close while buffering does not wedge loader threads

### Phase 3: Player UI and PiP

### Goal

Ship a usable fullscreen player surface.

### Work

- Add Media3 `PlayerView` or Compose wrapper UI
- Add buffering and stalled-state messaging
- Add unsupported-format / playback-failed messaging
- Enable PiP for active video playback
- Enter PiP on Home/gesture backgrounding when appropriate

### Manifest / activity concerns

- `supportsPictureInPicture`
- activity resize handling
- keeping playback and media session alive while in PiP

### Verification

- While video is playing, pressing Home enters PiP on supported Android versions
- Expanding from PiP returns to `PlayerActivity`
- Leaving the player intentionally stops playback and clears streaming demand

### Phase 4: Stability and Test Coverage

### Goal

Cover the failure modes that are easy to miss in happy-path manual testing.

### Minimum tests

- Unit:
  - file type routing
  - playback preparation policy
  - `TorrentDataSource` range-to-piece mapping
  - close/cancel behavior
- Instrumented:
  - tap video file opens player activity
  - buffering state renders while file is incomplete
  - PiP transition works without crashing
- E2E/manual:
  - incomplete MP4 playback from live torrent swarm
  - seek while incomplete
  - paused torrent resumed by play action
  - zero-peer stall path

## Risks

### 1. Media3 loader behavior under long waits

Blocking reads are allowed, but the implementation has to be disciplined. A sloppy `DataSource` can create hangs, retry storms, or leaked demand state.

### 2. Seek on sparse/incomplete content

Simple sequential playback may work immediately. Aggressive seek behavior is the first place where demand cancellation and reprioritization must be correct.

### 3. Device-specific codec support

Native Android will likely support more content than desktop Chrome, but support is still device-specific. The MVP needs clear error handling when a device cannot decode a stream natively.

### 4. Service lifecycle and backgrounding

PiP means playback may continue while the main UI is no longer foreground. The engine/service lifecycle must not accidentally tear down active playback.

## Deferred Follow-Ups

These should be separate phases after the MVP works:

- Optional MKV cue parsing for faster seek metadata
- Native codec capability probing and fallback policy
- HLS/remux path if progressive playback hits hard limits
- WebView fallback for unsupported files
- ffmpeg audio-only fallback
- Piece timeline / richer playback diagnostics

## Recommended File Layout

Likely new Android files:

- `android/app/src/main/java/com/jstorrent/app/player/PlayerActivity.kt`
- `android/app/src/main/java/com/jstorrent/app/player/TorrentPlaybackCoordinator.kt`
- `android/app/src/main/java/com/jstorrent/app/player/TorrentDataSource.kt`
- `android/app/src/main/java/com/jstorrent/app/player/TorrentDataSourceFactory.kt`
- `android/app/src/main/java/com/jstorrent/app/player/VideoFileDetector.kt`

Likely existing files to modify:

- [TorrentDetailScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentDetailScreen.kt)
- [FilesTab.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/tabs/FilesTab.kt)
- [AndroidManifest.xml](/Users/kgraehl/code/jstorrent/android/app/src/main/AndroidManifest.xml)

## MVP Exit Criteria

This MVP is done when all of the following are true:

- Tapping an incomplete video file opens `PlayerActivity`
- The file starts downloading if needed and is treated as the active playback target
- Media3 plays at least common progressive video files from incomplete torrents
- The app can seek, buffer, and recover without freezing
- PiP works while playback is active
- Failure states are understandable to the user

At that point, the native-first Android streaming story is proven. Further work becomes compatibility and polish, not basic feasibility.
