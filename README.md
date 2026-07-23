<img src="extension/public/icons/js-128.png" alt="JSTorrent" width="64" align="left" style="margin-right: 16px;">

# JSTorrent

A modern, full-featured BitTorrent client built on a shared TypeScript engine that runs everywhere.

**[Google Play](https://play.google.com/store/apps/details?id=com.jstorrent.app)** | **[Desktop App](https://jstorrent.com)** | **[Chrome Web Store](https://chromewebstore.google.com/detail/jstorrent/dbokmlpefliilbjldladbimlcfgbolhk)** | **[Website](https://jstorrent.com)**

## Platforms

| Platform | Status | Notes |
|----------|--------|-------|
| **Desktop App** | ✅ Available | Standalone app for macOS, Windows, and Linux ([download](https://jstorrent.com)) |
| **Chrome Extension** | ✅ Available | [Chrome Web Store](https://chromewebstore.google.com/detail/jstorrent/dbokmlpefliilbjldladbimlcfgbolhk) — Chrome, Edge, Brave, and other Chromium browsers |
| **Android** | ✅ Available | [Google Play](https://play.google.com/store/apps/details?id=com.jstorrent.app) — native app with QuickJS engine |
| **ChromeOS** | ✅ Available | [Extension](https://chromewebstore.google.com/detail/jstorrent/dbokmlpefliilbjldladbimlcfgbolhk) + [Android companion app](https://play.google.com/store/apps/details?id=com.jstorrent.app), or [Crostini setup help](https://jstorrent.com/#no-play-store) |
| **iOS** | ✅ Available | [AltStore PAL](https://altstore.io) (EU & Japan) or [sideload](https://github.com/kzahel/JSTorrent/releases) |

## Architecture

One TypeScript BitTorrent engine powers all platforms. Platform-specific native code handles networking and disk I/O, while the core protocol logic remains shared and tested across environments.

- **Desktop**: Tauri (Rust) — lightweight native window, no Electron. Small download, low memory footprint.
- **Android**: Native Kotlin + Jetpack Compose UI. The engine runs in-process via QuickJS with JNI bindings for I/O.
- **iOS**: Native SwiftUI + JavaScriptCore. The engine runs in-process with native TCP/UDP via Network.framework.
- **Extension**: Runs in the browser with a Rust sidecar (io-daemon) for disk and network access.
- **Node / CLI**: Node.js host and command-line tooling built on the same engine for daemon, streaming, and test workflows.

## Features

### BitTorrent Protocol
- ✅ Full BitTorrent protocol implementation
- ✅ Protocol encryption (MSE/PE)
- ✅ Web seeds from `.torrent` `url-list` and magnet `ws` sources (BEP 19)
- ✅ Seeding and leeching
- ✅ Tit-for-tat choking algorithm
- ✅ Optimistic unchoking
- ✅ Rarest-first piece selection
- ✅ Endgame mode
- ✅ Request pipelining
- ✅ SHA1 piece verification
- ✅ Fast extension (BEP 6)
- ✅ Extension protocol (BEP 10)
- ✅ Metadata exchange / magnet resolution (BEP 9)

### Networking
- ✅ UPnP port mapping
- ✅ DHT (Distributed Hash Table)
- ✅ PEX (Peer Exchange)
- ✅ UDP and HTTP trackers
- ✅ IPv4 and IPv6

### Performance
- ✅ Native host for fast networking and disk I/O
- ✅ File skipping and priorities
- ✅ Streaming-aware piece prioritization for playback
- ✅ Bandwidth throttling
- ✅ Connection limits

### User Experience
- ✅ Traditional torrent client UI
- ✅ Customizable interface
- ✅ Super responsive
- ✅ Dark mode
- ✅ Drag and drop torrents
- ✅ Click magnet links to add
- ✅ Stream media before download completes
- ✅ Installable search plugins for in-app torrent search
- ✅ Per-torrent and global statistics

## About

JSTorrent started as a [Chrome App](https://github.com/kzahel/jstorrent-legacy-app), was rebuilt as a Chrome Extension when Apps were deprecated, and has since expanded to Android and desktop platforms—all sharing the same TypeScript engine.

Written in TypeScript with comprehensive test coverage, including integration tests against [libtorrent](https://github.com/arvidn/libtorrent).

## Development

See [DEVELOPMENT.md](DEVELOPMENT.md) for build instructions and project structure.

## Documentation

- [Documentation](docs/README.md) - Living topics, normative contracts, protocol
  references, and historical material
- [Search Plugins](docs/topics/search-plugins.md) - Manifest structure, runtime
  API, result format, and the Internet Archive reference plugin

## License

MIT
