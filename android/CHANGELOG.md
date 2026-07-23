# Android Changelog

All notable changes to the Android app are documented here.

## [1.0.24] - 2026-07-23

### Changed
- Target Android 16 (API level 36) for the 2026 Google Play requirements
- Update Android build and test compatibility for API level 36

## [1.0.23]

### Added
- Media streaming player (Media3) with subtitle support, fullscreen swipe, and LAN sharing
- Torrent search with plugin system and WebView sandbox
- Web seed support (BEP 17/19) with keep-alive, concurrency, and redirect handling
- Configurable active piece memory limit
- Companion mode power management to prevent ARCVM Doze stalls
- Default gateway detection for port mapping (NAT-PMP/PCP)
- Data Saver detection with user warning banner

### Fixed
- Fix companion write backpressure deadlock: downloads stalled after 32MB of writes due to cumulative byte counter used for backpressure instead of in-flight bytes
- Fix OOM crash on Chromebox (Android 13, SDK 33)
- Use positioned SAF fd I/O for reduced write copy overhead
- Fix player pause behavior and fullscreen overlay controls
- Fix file priority await and .parts materialization on file unskip
- Prefer UTF-8 torrent metadata and paths

### Changed
- Add IO daemon conformance gate with contract versioning
- Harden Android memory crash paths and add instrumentation

## [1.0.22]

### Added
- In-app language picker with 18 tier 1 translations
- Manual theme setting (light/dark/system) in Advanced Settings
- Batch delete for faster torrent data removal

### Fixed
- SAF: fix `exists()` for directories, duplicate directory race condition, harden dir creation and cache validation
- Fix `removeTorrentWithData` skipping multi-file deletion and checking wrong root dir
- Fix per-app language selector not applying locale changes
- Fix JNI callbacks aborting on OOM (now throws JS exception)
- Fix deprecated edge-to-edge APIs for Play Store compliance
- Fix UPnP status display
- Engine: add TTL for cached failed file opens

### Changed
- Stop torrent network immediately on removal
- Move all hardcoded English strings to strings.xml for localization

## [1.0.21]

### Fixed
- Fix .torrent file opening crash (TransactionTooLargeException) by writing torrent bytes to temp file instead of passing base64 via intent extra
- Fix activity flags: remove FLAG_ACTIVITY_CLEAR_TASK to avoid destroying running activity, add FLAG_GRANT_READ_URI_PERMISSION for content:// URIs
- Show proper filenames from content providers instead of opaque document IDs

### Added
- Instant pending torrent placeholder while engine starts (for both .torrent files and magnet links)

## [1.0.20]

### Fixed
- Fix UDP hostname resolution failing on IPv4-bound sockets

## [1.0.19] - 2026-02-16

### Changed
- Reduced memory pressure to prevent OOM crashes during endgame
  - Max buffered bytes: 128MB → 64MB
  - Endgame duplicate requests: 3 → 2
  - Default pipeline depth: 500 → 250 (hidden from settings)
  - Piece buffer pool now scales inversely with piece size (16MB cap)

## [1.0.18] - 2026-02-16

### Added
- `listTree` API for recursive file listing, used in resume/recheck for batched file existence checks
- `verifyChunks` API for batch piece hash verification

### Fixed
- Android standalone always deleting torrent data on removal
- QuickJS FFI boolean coercion bugs in native filesystem

## [1.0.16] - 2026-02-10

### Changed
- Reduced log verbosity (tracker, tick, maintenance, and backpressure diagnostics moved to debug level)
- Config storage simplified to local-only
- Notifications and keep-awake settings now available on all platforms

## [1.0.15] - 2026-02-08

### Added
- Queue management UI and settings
- Log viewer
- File opening support
- Completion notifications
- Seed rotation and reset command
- Async disk reads and upload diagnostics
- Disk I/O layer plumbing (diskId across platforms)

### Fixed
- Instrumented test failures (missing resetTorrent and null-mode session restore)
- Recheck race in torrent file ops

### Changed
- Tick bottleneck diagnostics and reduced active pieces
- Simplified test helpers and improved companion server shutdown
- Network detection improvements

## [1.0.14] - 2026-02-05

### Fixed
- TCP data reordering race condition that caused download stalls
- Disconnect peers sending invalid message lengths (>1MB)

### Changed
- Restored endgame piece requesting for faster download completion
- Handler queue tracking for active pieces

## [1.0.13] - 2026-02-05

### Fixed
- ForegroundServiceDidNotStartInTimeException crash on Android 12+
- Foreground service lifecycle on rapid stop/start cycles
- Subscription lifecycle across engine restarts

### Changed
- Show network waiting status in torrent list
- Auto-pause new torrents when network is restricted
- Navigate to torrent list after clearing all data

## [1.0.12] - 2026-02-04

### Fixed
- Engine startup race conditions with command queueing
- Stale cache entries after torrent removal
- Race conditions in batch operations (pause/resume/remove selected)

## [1.0.11] - 2026-02-04

### Added
- Torrent recheck functionality (verify and re-download corrupted pieces)
- File truncation support for accurate file sizes
- Conservative seeding mode with pending read limit
- Worker hasher for improved hashing performance

### Fixed
- Startup race conditions
- Connection race conditions during torrent removal

## [1.0.10] - 2026-02-03

### Changed
- Enabled R8 minification and resource shrinking (83% smaller APK, ~9.5MB vs ~55MB)
- Added 16KB memory page size support for Android 15+ devices

## [1.0.9] - 2026-02-03

### Added
- Feedback/bug report feature

## [1.0.8] - 2026-01-31

### Added
- Torrent metadata display (file list, size, piece info)
- Improved settings UX

### Changed
- Simplified adaptive batching for better performance
- Increased disk worker throughput

## [1.0.6]

### Added
- Background service with lazy engine startup
- Torrent summary cache for faster app launch
- Engine status indicator in UI

### Changed
- Improved app lifecycle and service management
- Better TCP socket performance

## [1.0.5]

### Added
- Batch verified writes for reduced FFI overhead
- Zero-copy PIECE message handling

### Fixed
- Binary data encoding between JS engine and Kotlin

## [1.0.4]

### Added
- NIO-based TCP implementation
- Pooled file handles for better I/O

### Changed
- Improved tick loop timing and metrics

## [1.0.3]

### Added
- Initial Play Store release
- BitTorrent v1 and v2 support
- Magnet link handling
- DHT, PEX, and tracker support
