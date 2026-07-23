# iTorrent Implementation Reference

Source: https://github.com/XITRIX/iTorrent — cloned to `~/code/reference/iTorrent`

## Background Execution

iTorrent offers two user-selectable background modes:

### Audio Mode (Default)

**Files:** `iTorrent/Services/BackgroundService/AudioBackgroundService.swift`

- Plays a silent audio file (`sound.m4a`) at volume 0.01 in an infinite loop
- Uses `AVAudioSession` with category `.playback` and `.mixWithOthers`
- Combines with `UIApplication.beginBackgroundTask()` that renews every ~10 seconds
- Declared in Info.plist: `UIBackgroundModes: ["audio"]`
- No user-visible indicator (unlike location mode)
- Handles `AVAudioSession.interruptionNotification` to auto-resume if interrupted

### Location Mode (Alternative)

**Files:** `iTorrent/Services/BackgroundService/LocationBackgroundService.swift`

- `CLLocationManager` with `allowsBackgroundLocationUpdates = true`
- Requests `.requestAlwaysAuthorization()`
- Uses `kCLLocationAccuracyReduced` (coarse location only)
- Disables `pausesLocationUpdatesAutomatically`
- Shows blue location indicator in status bar
- Gated behind compile flag `IS_SUPPORT_LOCATION_BG`

### Background Service Coordinator

**Files:** `iTorrent/Services/BackgroundService/BackgroundService.swift`

- Protocol-based: `BackgroundServiceProtocol` with `start()`, `stop()`, `isRunning`
- Auto-starts on `sceneDidEnterBackground` if torrents are actively downloading
- Auto-stops when all torrents complete (no unnecessary background)
- `BGTaskScheduler` used only for RSS feed polling, NOT for torrent I/O

### Info.plist Background Modes

```xml
<key>UIBackgroundModes</key>
<array>
    <string>audio</string>        <!-- Silent audio keep-alive -->
    <string>fetch</string>         <!-- RSS background fetch -->
    <string>processing</string>    <!-- BGProcessingTask -->
</array>
```

## Architecture Overview

- **UI:** UIKit (not SwiftUI), custom MVVM framework (`MvvmFoundation`)
- **Engine:** libtorrent v2.0.10 (C++ library) via precompiled `LibTorrent.framework`
- **Networking:** All TCP/UDP/DHT handled by libtorrent natively — no Network.framework for peer connections
- **Reactive:** Combine framework for state propagation
- **Package manager:** Git submodules for dependencies

## Key Dependencies

- `LibTorrent.framework` + `OpenSSL` (C++ torrent engine)
- `MvvmFoundation` (custom MVVM)
- `GCDWebServer` (WebDAV file sharing)
- `SWXMLHash` (RSS parsing)
- Firebase (analytics/crashlytics)

## Project Structure

```
iTorrent/
├── Core/           AppDelegate, SceneDelegate, lifecycle
├── Services/       TorrentService, BackgroundService, Preferences
├── Screens/        63 view controllers/view models
├── Components/     Reusable UI components
├── Utils/          Extensions, helpers
└── Assets/         Images, configs, sound.m4a

ProgressWidget/     iOS 16+ Live Activity (Dynamic Island)
Submodules/         LibTorrent-Swift, GoogleAdsSdk, etc.
```

## Notable Features

- Live Activities / Dynamic Island for download progress (iOS 16+)
- WebDAV server for file access from other devices
- RSS feed auto-download
- Multiple storage locations
- Magnet links + .torrent file import
- Proxy support (SOCKS5)

## Relevance to JSTorrent iOS

- **Background audio trick**: Same pattern we use in the Chrome extension for background tab execution. Can be added as a toggle later.
- **Architecture differs**: iTorrent uses C++ libtorrent; we'll use our TypeScript engine via JavaScriptCore. But the background execution strategy is platform-level and engine-agnostic.
- **UIKit vs SwiftUI**: iTorrent chose UIKit. We're going SwiftUI for the MVP since our UI is minimal.

## Background Audio — Legitimacy Notes

The silent audio hack is the most practical background approach. Key advantages over location mode:
- No permission prompt required (audio session is a normal app capability)
- No visible system indicator (location mode shows the blue dot)
- Extremely common pattern across iOS apps (sleep timers, ambient sound apps, podcast apps)

Can be made more legitimate by providing actual audio feedback — e.g. subtle sounds on piece completion, a chime on torrent completion, or an ambient "data transfer" sound effect. This makes the `audio` background mode genuinely purposeful rather than purely a keep-alive hack. User controls the volume / mute as a setting.
