# JSTorrent Everywhere: Architecture Vision

**Date:** December 2025  
**Status:** Implemented  
**Author:** Kyle / Claude

---

## Vision

JSTorrent is the torrent client that truly runs everywhere, powered by a single TypeScript engine with platform-native I/O bindings.

```
┌────────────────────────────────────────────────────────────────────┐
│                    @jstorrent/engine (TypeScript)                  │
│                                                                     │
│    The same BitTorrent protocol code runs on every platform.       │
│    Only the I/O layer differs.                                     │
└────────────────────────────────────────────────────────────────────┘
                                  │
    ┌──────────────┬──────────────┼──────────────┬──────────────┬──────────────┐
    │              │              │              │              │              │
    ▼              ▼              ▼              ▼              ▼              ▼
┌────────┐  ┌────────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐
│ Chrome │  │ Any Browser│  │  Tauri   │  │ QuickJS  │  │   JSC    │  │ Chrome   │
│  ext   │  │ (no ext)   │  │ WebView  │  │          │  │          │  │  ext     │
│        │  │            │  │          │  │          │  │          │  │          │
│Desktop │  │ jstorrent  │  │ Desktop  │  │ Android  │  │   iOS    │  │ ChromeOS │
│Rust IO │  │ .com +     │  │standalone│  │standalone│  │standalone│  │Kotlin IO │
│        │  │ Rust IO    │  │ Rust IO  │  │          │  │          │  │          │
└────────┘  └────────────┘  └──────────┘  └──────────┘  └──────────┘  └──────────┘
```

**Why this matters:**

- **One codebase** for BitTorrent protocol, piece management, peer connections, DHT, trackers
- **Native performance** where it counts: socket I/O, file operations, hashing
- **Platform-native UI** for each target (Compose on Android, SwiftUI on iOS, web on desktop)
- **Background execution** on mobile via native services

---

## Platform Configurations

| Platform | JS Runtime | I/O Layer | UI | Distribution |
|----------|------------|-----------|-----|--------------|
| Desktop (Linux/Win/Mac) | Chrome V8 (extension) | Rust native host | Web (React/Solid) | Chrome Web Store + installers |
| Desktop (any browser) | Any browser V8* | Rust native host | Web (React/Solid) | jstorrent.com + installers |
| Desktop (standalone) | Tauri webview | Rust (Tauri commands) | Web (React/Solid) | Direct download (Tauri installer) |
| ChromeOS | Chrome V8 (extension) | Kotlin companion | Web (React/Solid) | Chrome Web Store + Play Store |
| ChromeOS Flex** | Chrome V8 (extension) | Rust io-daemon (Crostini) | Web (React/Solid) | Chrome Web Store + install script |
| Android Standalone | QuickJS | Kotlin io-core | Jetpack Compose | Play Store |
| iOS | JavaScriptCore | Swift io-core | SwiftUI | App Store / AltStore / Sideload |

\* Works in Firefox, Edge, Brave, etc. via jstorrent.com connecting to localhost. Safari excluded (127.0.0.1 not a secure context). Safari users can use the standalone Tauri app instead.

\*\* ChromeOS Flex has no ARC (Android Runtime), so the companion app isn't available. The io-daemon runs in Crostini via a one-liner install script with systemd lingering. See [ChromeOS Flex roadmap](../roadmap/chromeos-flex.md).

---

## The Engine: Platform-Agnostic Core

The `@jstorrent/engine` package contains all BitTorrent logic with zero platform dependencies:

```
packages/engine/
├── src/
│   ├── core/           # BtEngine, Torrent, PeerConnection, Swarm
│   ├── protocol/       # Wire protocol, bencode, handshakes
│   ├── tracker/        # HTTP/UDP tracker clients
│   ├── dht/            # Distributed hash table
│   ├── storage/        # Piece management, file allocation
│   ├── interfaces/     # ISocketFactory, IFileSystem, ISessionStore, IHasher
│   └── adapters/       # Platform-specific implementations
│       ├── daemon/     # WebSocket to Rust/Kotlin daemon (extension mode)
│       ├── native/     # Direct native bindings (QuickJS/JSC mode)
│       ├── browser/    # Browser APIs (SubtleCrypto, localStorage)
│       ├── android/    # WebView bridges (legacy standalone)
│       ├── node/       # Node.js (testing)
│       ├── memory/     # In-memory (unit tests)
│       └── null/       # No-op implementations (testing)
```

### Interface Surface

The engine depends on four core interfaces:

```typescript
// packages/engine/src/interfaces/

interface ISocketFactory {
  createTcpSocket(host?: string, port?: number): Promise<ITcpSocket>
  createUdpSocket(bindAddr?: string, bindPort?: number): Promise<IUdpSocket>
  createTcpServer(): ITcpServer
}

interface IFileSystem {
  open(rootKey: string, path: string): Promise<IFileHandle>
}

interface IFileHandle {
  read(buffer: Uint8Array, offset: number, length: number, position: number): Promise<{bytesRead: number}>
  write(buffer: Uint8Array, offset: number, length: number, position: number): Promise<{bytesWritten: number}>
  close(): Promise<void>
}

interface ISessionStore {
  get(key: string): Promise<Uint8Array | null>
  set(key: string, value: Uint8Array): Promise<void>
  delete(key: string): Promise<void>
  keys(prefix?: string): Promise<string[]>
  getJson<T>(key: string): Promise<T | null>
  setJson<T>(key: string, value: T): Promise<void>
}

interface IHasher {
  sha1(data: Uint8Array): Promise<Uint8Array>
}
```

Every platform implements these four interfaces. The engine doesn't care how.

---

## Native Adapter: Unified Binding Interface

For QuickJS (Android) and JavaScriptCore (iOS), we define a unified native binding contract:

```typescript
// packages/engine/src/adapters/native/bindings.d.ts

declare global {
  // TCP
  function __jstorrent_tcp_connect(socketId: number, host: string, port: number): void
  function __jstorrent_tcp_send(socketId: number, data: ArrayBuffer): void
  function __jstorrent_tcp_close(socketId: number): void
  function __jstorrent_tcp_on_data(callback: (socketId: number, data: ArrayBuffer) => void): void
  function __jstorrent_tcp_on_close(callback: (socketId: number, hadError: boolean) => void): void
  function __jstorrent_tcp_on_error(callback: (socketId: number, message: string) => void): void
  function __jstorrent_tcp_on_connected(callback: (socketId: number, success: boolean) => void): void

  // UDP
  function __jstorrent_udp_bind(socketId: number, addr: string, port: number): void
  function __jstorrent_udp_send(socketId: number, addr: string, port: number, data: ArrayBuffer): void
  function __jstorrent_udp_close(socketId: number): void
  function __jstorrent_udp_on_message(callback: (socketId: number, addr: string, port: number, data: ArrayBuffer) => void): void
  function __jstorrent_udp_on_bound(callback: (socketId: number, success: boolean, port: number) => void): void

  // Files
  function __jstorrent_file_open(handleId: number, rootKey: string, path: string): void
  function __jstorrent_file_read(handleId: number, offset: number, length: number): ArrayBuffer
  function __jstorrent_file_write(handleId: number, offset: number, data: ArrayBuffer): number
  function __jstorrent_file_close(handleId: number): void

  // Hashing
  function __jstorrent_sha1(data: ArrayBuffer): ArrayBuffer

  // Storage (SharedPreferences / UserDefaults)
  function __jstorrent_storage_get(key: string): string | null
  function __jstorrent_storage_set(key: string, value: string): void
  function __jstorrent_storage_delete(key: string): void
  function __jstorrent_storage_keys(prefix: string): string  // JSON array

  // Text encoding (QuickJS lacks TextEncoder/TextDecoder)
  function __jstorrent_text_encode(str: string): ArrayBuffer
  function __jstorrent_text_decode(data: ArrayBuffer): string

  // Timers (QuickJS has setTimeout but not setInterval)
  function __jstorrent_set_timeout(callback: () => void, ms: number): number
  function __jstorrent_clear_timeout(id: number): void

  // Crypto
  function __jstorrent_random_bytes(length: number): ArrayBuffer
}
```

Both Kotlin (for Android) and Swift (for iOS) implement these identical function signatures. The TypeScript adapter doesn't know or care which platform it's running on.

---

## Android Architecture

### Base Structure (Companion Mode Only)

```
android/                          # Renamed from android-io-daemon
├── io-core/                      # Pure I/O primitives
│   └── com/jstorrent/io/
│       ├── socket/
│       │   ├── TcpSocketManager.kt
│       │   ├── UdpSocketManager.kt
│       │   └── TcpServerManager.kt
│       ├── file/
│       │   └── FileManager.kt
│       └── hash/
│           └── Hasher.kt
│
├── companion-server/             # HTTP/WS server for extension mode
│   └── com/jstorrent/companion/
│       ├── CompanionHttpServer.kt    # Ktor server setup
│       ├── IoWebSocketHandler.kt     # WebSocket /io endpoint
│       └── FileRoutes.kt             # HTTP /read, /write endpoints
│
└── app/                          # Main application
    └── com/jstorrent/app/
        ├── MainActivity.kt
        ├── StandaloneActivity.kt     # WebView standalone (debug)
        └── mode/
            └── ModeManager.kt
```

### Current State (With QuickJS)

```
android/
├── io-core/                      # Pure I/O primitives (unchanged)
│
├── companion-server/             # HTTP/WS server (unchanged)
│
├── quickjs-engine/               # QuickJS runtime module
│   ├── build.gradle.kts
│   ├── src/main/
│   │   ├── kotlin/com/jstorrent/quickjs/
│   │   │   ├── QuickJsEngine.kt        # QuickJS context wrapper
│   │   │   ├── JsThread.kt             # Dedicated single JS thread
│   │   │   ├── EngineController.kt     # Start/stop/status API
│   │   │   └── bindings/
│   │   │       ├── NativeBindings.kt   # Registers __jstorrent_* functions
│   │   │       ├── TcpBindings.kt      # TCP socket bindings
│   │   │       ├── UdpBindings.kt      # UDP socket bindings
│   │   │       ├── FileBindings.kt     # File I/O bindings
│   │   │       ├── StorageBindings.kt  # SharedPreferences bindings
│   │   │       └── PolyfillBindings.kt # TextEncoder, timers, etc.
│   │   ├── jni/                        # QuickJS CMake build (from submodule)
│   │   └── assets/
│   │       └── engine.bundle.js        # Bundled engine code
│   └── src/test/                       # Unit tests
│
└── app/
    └── com/jstorrent/app/
        ├── MainActivity.kt             # Companion mode entry
        ├── StandaloneActivity.kt       # WebView standalone (debug)
        ├── NativeStandaloneActivity.kt # Compose UI + QuickJS engine
        │   └── (contains composables: NativeStandaloneScreen,
        │       TorrentCard, AddTorrentRow, SetupRequiredCard)
        ├── service/
        │   └── EngineService.kt        # Foreground service for QuickJS
        └── mode/
            └── ModeDetector.kt         # Chromebook vs Android detection
```

### Android App Modes

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Android App Modes                           │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐     │
│  │   Companion     │  │    WebView      │  │     Native      │     │
│  │     Mode        │  │   Standalone    │  │   Standalone    │     │
│  ├─────────────────┤  ├─────────────────┤  ├─────────────────┤     │
│  │ companion-server│  │ companion-server│  │ quickjs-engine  │     │
│  │ (HTTP/WS)       │  │ + WebView UI    │  │ + Compose UI    │     │
│  │                 │  │ + injected auth │  │                 │     │
│  ├─────────────────┤  ├─────────────────┤  ├─────────────────┤     │
│  │    io-core      │  │    io-core      │  │    io-core      │     │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘     │
│                                                                     │
│  Engine location:     Engine location:     Engine location:        │
│  Chrome extension     WebView (V8)         QuickJS                 │
│                                                                     │
│  Use case:            Use case:            Use case:               │
│  ChromeOS pairing     Debug/test           Production standalone   │
└─────────────────────────────────────────────────────────────────────┘
```

### Thread Model (Android Native Standalone)

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  QuickJS Thread │     │  IO Thread Pool │     │  Main Thread    │
│                 │     │  (Coroutines)   │     │  (Android UI)   │
├─────────────────┤     ├─────────────────┤     ├─────────────────┤
│ Engine logic    │◄───►│ Socket I/O      │     │ Compose UI      │
│ Piece mgmt      │     │ File I/O        │     │ Notifications   │
│ Peer protocol   │     │ DNS resolution  │     │ Service binding │
│ DHT             │     │ Hashing         │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
        │                       │                       │
        └───────────────────────┴───────────────────────┘
                    JNI calls (thread-safe)
```

**Critical:** QuickJS is single-threaded. All JS execution happens on one dedicated thread. Native callbacks must post results back to the JS thread, never call directly from I/O threads.

---

## iOS Architecture (Future)

```
ios/
├── io-core/                      # Swift I/O primitives
│   ├── Sources/IOCore/
│   │   ├── TcpSocketManager.swift
│   │   ├── UdpSocketManager.swift
│   │   ├── FileManager.swift
│   │   └── Hasher.swift
│   └── Package.swift
│
├── jsc-bridge/                   # JavaScriptCore bindings
│   ├── Sources/JSCBridge/
│   │   ├── JSCRuntime.swift          # Lifecycle, script loading
│   │   ├── NativeBindings.swift      # Registers __jstorrent_* functions
│   │   └── EngineController.swift    # Start/stop/status API
│   └── Package.swift
│
└── app/                          # Main iOS app
    ├── JSTorrent.xcodeproj
    └── Sources/
        ├── JSTorrentApp.swift
        ├── ContentView.swift
        ├── TorrentListView.swift     # SwiftUI
        ├── FileListView.swift        # SwiftUI
        └── SettingsView.swift        # SwiftUI
```

### iOS vs Android Differences

| Aspect | Android (QuickJS) | iOS (JavaScriptCore) |
|--------|-------------------|----------------------|
| JS Runtime | QuickJS (external lib) | JavaScriptCore (built into iOS) |
| Native bindings | JNI + Kotlin | Swift JSExport protocol |
| Background execution | Foreground Service | Background App Refresh (limited) |
| File storage | SAF / app private | App container |
| Distribution | Play Store | App Store / AltStore / Sideload |

**Note:** iOS background execution is more restricted. Downloads may pause when app is backgrounded unless using specific entitlements (background audio, VoIP, etc.). This is a known limitation.

---

## Desktop Architecture (Current)

```
desktop/                          # Renamed from system-bridge
├── common/                       # Shared Rust code
├── host/                         # jstorrent-host (native messaging coordinator)
├── io-daemon/                    # I/O operations (sockets, files, hashing)
├── link-handler/                 # OS protocol handler (magnet:, .torrent)
├── installers/                   # Platform-specific installers
│   ├── windows/                  # NSIS/Inno Setup
│   ├── macos/                    # pkgbuild
│   └── linux/                    # deb/AppImage
└── manifests/                    # Chrome native messaging manifests
```

Desktop remains extension-based. The engine runs in Chrome's V8, I/O happens via native messaging to Rust binaries.

---

## Bundle Compilation Pipeline

The engine TypeScript must be bundled into a single JS file for QuickJS/JSC:

```
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│ TypeScript   │───►│ JavaScript   │───►│ .js file     │
│ source       │    │ bundle       │    │ (ES2020)     │
└──────────────┘    └──────────────┘    └──────────────┘
     esbuild            (single file,      assets/
                         no browser APIs)
```

### Build Configuration

```
packages/engine/
├── bundle/
│   ├── esbuild.native.config.js  # Bundle config for native adapters
│   └── build-native.js           # Entry point
├── src/adapters/native/
│   ├── bindings.d.ts             # Type declarations for __jstorrent_*
│   ├── socket-factory.ts
│   ├── filesystem.ts
│   ├── session-store.ts
│   ├── hasher.ts
│   └── index.ts                  # Entry point that wires everything
```

### npm Script

```json
// packages/engine/package.json
{
  "scripts": {
    "bundle:native": "node bundle/build-native.js"
  }
}
```

### Gradle Integration (Android)

```kotlin
// android/quickjs-engine/build.gradle.kts

tasks.register("buildEngineBundle") {
    doLast {
        exec {
            workingDir = rootProject.file("../../packages/engine")
            commandLine("pnpm", "bundle:native")
        }
        copy {
            from(rootProject.file("../../packages/engine/dist/engine.native.js"))
            into("src/main/assets")
            rename { "engine.bundle.js" }
        }
    }
}

tasks.named("preBuild") {
    dependsOn("buildEngineBundle")
}
```

---

## Distribution Strategy

### The "No Storefront" Problem

The Chrome extension has a natural trust anchor (Chrome Web Store) with reviews, ratings, and Google's approval. The Tauri desktop app lacks this — it's distributed via GitHub Releases with no centralized discovery or social proof.

### Package Managers (Trust + Discovery)

| Platform | Package Manager | Notes |
|----------|----------------|-------|
| macOS | Homebrew cask | High trust in developer community. Submit formula via PR, automate updates with `brew bump-cask-pr` in release CI. |
| Windows | winget | Ships with Windows 10/11. Submit manifest via PR to `microsoft/winget-pkgs`, automate with `komac` in CI. |
| Linux | Flatpak / Snap / Homebrew | Broader reach. Flatpak for sandboxed desktop apps. |

CI automation: after release artifacts are published, a final job submits PRs to update the cask/winget manifest (version + SHA256). Initial submission is manual; version bumps are automated.

### Microsoft Store (Paid Support Channel)

Win32 apps can be packaged as MSIX and submitted to the Microsoft Store with minimal sandboxing restrictions. Torrent clients are allowed.

- 15% revenue share
- Use as a "support the author" paid listing — identical to the free version
- Store-managed auto-updates as a minor perk
- Lower friction than Mac App Store — no sandboxing concerns for networking

### Remote Control & App Store Strategy

#### Built-in Remote Control (All Platforms)

Every JSTorrent instance (desktop, Android, headless) exposes a remote control API. This is a first-class feature, not a workaround for store policies.

```
┌─────────────────┐         ┌───────────────────┐         ┌─────────────────┐
│  Remote Control │         │   SRP Relay        │         │  JSTorrent      │
│  App            │◄───────►│   (WebSocket)      │◄───────►│  Instance       │
│  (Mac/iOS/web)  │  E2E    │                    │  E2E    │  (any platform) │
└─────────────────┘  enc.   └───────────────────┘  enc.   └─────────────────┘
```

**Protocol:**
- WebSocket transport via SRP relay (NAT traversal, no port forwarding)
- SRP-6a authentication (zero-knowledge password proof — relay never sees credentials)
- E2E encryption (TweetNaCl — relay is a dumb pipe, cannot read traffic)
- HTTP-like request/response multiplexed over the encrypted channel
- Auto-detects local instances and connects directly (skips relay)

**Relay infrastructure:** Existing `yepanywhere` relay server (Node.js, `ws`, `better-sqlite3`). Can self-host or use a hosted relay.

#### Mac App Store

Apple does not allow torrent clients — only remote control apps. Because remote control is a genuine feature of every JSTorrent instance, a **native SwiftUI remote control app** is defensible:

- Connects to any JSTorrent instance (local or remote) via the same SRP relay protocol
- Auto-discovers local running instance, connects directly without relay
- Manages multiple instances (home desktop, headless server, Android phone)
- Works standalone for library management / torrent metadata browsing
- "For full download capability, install JSTorrent from jstorrent.com" — links to website are fine
- Desktop daemon also installable via `brew install jstorrent`
- Free listing for discovery, or paid as a "support the author" purchase
- 30% cut (15% under Small Business Program < $1M/year)

**Why this stands out:** Most torrent remote-control apps in the Mac App Store are for controlling seedboxes. JSTorrent's remote control talks to locally-installed instances too — making it the closest thing to a native torrent client on the Mac App Store.

#### iOS App Store

Same app, same justification. A SwiftUI remote control app for iPhone/iPad that controls any JSTorrent instance via the relay. Useful for monitoring downloads, adding torrents on the go, managing a headless instance.

#### Microsoft Store

Win32 apps can be packaged as MSIX with minimal sandboxing restrictions. Torrent clients are allowed directly.

- 15% revenue share
- Use as a "support the author" paid listing — identical to the free version
- Store-managed auto-updates as a minor perk

### Chrome Extension

The extension has the strongest distribution story: Chrome Web Store provides discovery, reviews, and Google's trust anchor. The desktop app offers browser independence and background downloads without Chrome running. Different users will prefer different form factors.

---

## References

- [QuickJS](https://bellard.org/quickjs/) - Fabrice Bellard's lightweight JS engine
- [quickjs-ng](https://github.com/quickjs-ng/quickjs) - Actively maintained QuickJS fork
- [JavaScriptCore](https://developer.apple.com/documentation/javascriptcore) - Apple's JS engine
