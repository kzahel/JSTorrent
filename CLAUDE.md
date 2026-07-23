# Claude Instructions

## Cross-Project Context

For cross-project context (how this project relates to Transistor, QuickJitJS, Tilefun, etc.), see `~/code/dotfiles/projects/README.md`.

## Living Topic Documentation

Focused, living records of continuing concerns live under `docs/topics/`.
Before changing a continuing concern, look for a relevant topic and read it.
Update that topic when the work changes its status, decisions, evidence,
validation, gaps, or recommended direction. Create a focused sibling topic when
continuity would be valuable and the concern can evolve independently.

Adopt this convention incrementally: do not reorganize existing documentation
solely for consistency, and do not create a topic for every small standalone
change. See `docs/topics/README.md` for document roles and topic shape. When a
commit materially advances a documented topic, use its slug in a
`Topic: <slug>` commit-message trailer where practical.

## Environment Setup

Before running commands that require Java, Rust, or other development tools, source the shell profile:

```bash
source ~/.profile
```

This loads PATH entries for:
- Java (required for Android/Gradle builds)
- Rust/Cargo
- Other development tools

### Local `playsvideo` Development

When developing JSTorrent against a local `playsvideo` checkout, keep the committed dependency on the published package for CI compatibility, then relink locally after installs:

```bash
pnpm install
pnpm --dir packages/client link /Users/kgraehl/code/playsvideo
```

Notes:
- `pnpm install` can replace the local link with the declared dependency, so rerun the link command when you want to resume local `playsvideo` development.
- If JSTorrent needs newer `playsvideo` exports, also make sure the local `playsvideo` checkout is built or running in watch mode.

## Product Deployments

JSTorrent ships as multiple products that share the same TypeScript engine but run in different configurations depending on platform.

### Deployment Matrix

```
┌──────────────────┬──────────────────┬───────────────┬───────────────┬──────────────┐
│ Platform         │ Product(s)       │ Engine runs   │ I/O backend   │ UI           │
│                  │                  │ in            │               │              │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ Mac/Win/Linux    │ Extension +      │ Extension UI  │ Rust          │ Extension    │
│ (with browser)   │ Desktop app *    │ page          │ io-daemon     │ (tab/popup)  │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ Mac/Win/Linux    │ Desktop app      │ Tauri webview │ Rust          │ Extension    │
│ (standalone)     │ alone            │               │ io-daemon     │ assets       │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ ChromeOS         │ Extension +      │ Extension UI  │ Android       │ Extension    │
│ (primary)        │ Android app **   │ page          │ companion     │ (tab/popup)  │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ ChromeOS         │ Android app      │ QuickJS       │ Kotlin JNI    │ Native       │
│ (standalone)     │ alone            │ (in-process)  │ (FileManager) │ Compose      │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ ChromeOS Flex    │ Extension +      │ Extension UI  │ Rust io-daemon│ Extension    │
│ (no ARC, adv.)   │ Crostini daemon  │ page          │ (in Crostini) │ (tab/popup)  │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ Android phone    │ Android app      │ QuickJS       │ Kotlin JNI    │ Native       │
│                  │                  │ (in-process)  │ (FileManager) │ Compose      │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ Any (npm)        │ CLI              │ Node.js       │ Node fs       │ Terminal     │
│                  │ @jstorrent/engine│               │               │              │
├──────────────────┼──────────────────┼───────────────┼───────────────┼──────────────┤
│ iOS (AltStore    │ iOS app          │ JavaScriptCore│ iOS native    │ Native       │
│ PAL, EU only)    │                  │ (in-process)  │ (FileManager) │ SwiftUI      │
└──────────────────┴──────────────────┴───────────────┴───────────────┴──────────────┘

*  Extension requires desktop app. Desktop app installs a native messaging host
   that auto-launches io-daemon — user doesn't need to interact with desktop app.
** Requires one-time pairing between extension and Android companion.
```

### Product Details

**Chrome Extension** (`extension/`): Opens as a tab or popup in Chrome. The engine runs in the foreground UI page (not the service worker). Requires either the desktop app (Mac/Win/Linux) or Android companion (ChromeOS) for I/O — it cannot function alone.

**Tauri Desktop App** (`desktop/tauri-app/`): Bundles the same extension UI assets into a native desktop window. Can run standalone (without the browser extension) or headlessly as the I/O backend for the extension. Provides auto-updates. The io-daemon runs as a sidecar process.

**Android App** (`android/`): Single APK, no build variants. Has its own native Compose UI. Runs a foreground service for background operation. Operates in two modes:
- **Standalone mode**: Engine runs in QuickJS in-process, full native UI. Used on phones and optionally on ChromeOS.
- **Companion mode**: Minimal UI (pair/unpair, mode switch). Runs the companion server (HTTP + WebSocket) so the Chrome extension can use it for I/O on ChromeOS.

**Node CLI** (`packages/engine/`): Published to npm as `@jstorrent/engine`. Primarily for integration testing; the Node adapters are the test/reference implementation.

**iOS App** (`ios/`): Native SwiftUI app with JavaScriptCore engine. Standalone — runs the full engine in-process with native TCP/UDP via Network.framework. Distributed via AltStore PAL (EU) and sideloading (App Store rejects torrent clients). Downloads run in foreground only (no background service equivalent on iOS).

**Sandbox overview**: See `docs/architecture/sandbox-overview.md` for the current platform sandbox boundaries and search plugin / Google Play review notes.

### Pairing

- **Desktop**: No pairing needed. Native messaging host auto-launches io-daemon with a known token.
- **ChromeOS (extension + Android companion)**: One-time pairing flow required. Extension discovers companion, user confirms pairing dialog on Android side, tokens exchanged.

## Engine Architecture & Backends

The engine is a single TypeScript codebase (`packages/engine/`) that runs in three runtime modes, each with a different I/O backend. All backends implement the same `IFileSystem` / `IFileHandle` interfaces.

### Runtime Modes

```
┌────────────────────────────────────────────────────────────────────────┐
│                        TypeScript Engine                               │
│                     (packages/engine/src/)                             │
├────────────┬──────────────────┬──────────────────┬────────────────────┤
│  Preset:   │     node         │     daemon       │     native         │
│  Runs in:  │  Node.js CLI     │  Browser/Tauri   │  Android app       │
│            │                  │  (UI page)       │  (QuickJS via JNI) │
├────────────┼──────────────────┼──────────────────┼────────────────────┤
│ FileSystem │ ScopedNode       │ Daemon           │ Native             │
│            │ (Node fs API)    │ (HTTP to daemon) │ (JNI to Kotlin)    │
├────────────┼──────────────────┼──────────────────┼────────────────────┤
│ Networking │ Node net/dgram   │ WebSocket mux    │ WebSocket mux      │
│            │                  │ to io-daemon     │ to Kotlin bindings │
├────────────┼──────────────────┼──────────────────┼────────────────────┤
│ DiskQueue  │ TorrentDiskQueue │ TorrentDiskQueue │ Passthrough +      │
│            │ (6 JS workers)   │ (adaptive batch) │ NativeBatchingDQ   │
├────────────┼──────────────────┼──────────────────┼────────────────────┤
│ I/O target │ Local filesystem │ Rust io-daemon   │ Android SAF /      │
│            │                  │ OR Android       │ file:// I/O        │
│            │                  │ companion server │                    │
└────────────┴──────────────────┴──────────────────┴────────────────────┘
```

Additional test/benchmark adapters: `InMemoryFileSystem` (unit tests), `NullFileSystem` (benchmarks — discards writes).

### Storage Layer Hierarchy

```
StorageRootManager                    Manages roots, assigns torrents to roots
  ├─ roots: Map<key, StorageRoot>     Each root = {key, label, path, diskId?}
  ├─ torrentRoots: Map<hash, key>     Which root a torrent uses
  └─ fsCache: Map<key, IFileSystem>   One filesystem instance per root
       │
       ▼
TorrentContentStorage                 Per-torrent: maps pieces → files
  ├─ fileHandles: Map<path, IFileHandle>   Cached open handles
  ├─ failedPaths: Set<string>              Skip files that failed to open
  └─ diskQueue: IDiskQueue                 Concurrency control + batching
       │
       ▼
IFileSystem / IFileHandle             Backend-specific I/O (see adapters below)
```

**rootKey** is the stable identifier for a storage location. For Node, it's derived from the path. For daemon/native backends, it's passed to every I/O call so the backend can resolve which physical storage to use (e.g., SAF volume URI on Android).

For detailed backend implementation reference (IFileSystem interface, adapter table, HTTP endpoints, JNI bindings, Kotlin FileManager), see the Claude memory file `engine-backends.md`. That file may drift from the code — when in doubt, read the source directly.

### Adding a New IFileSystem Method (Checklist)

When adding a new method (e.g., `listTree`), implement in this order:

1. **Interface**: `packages/engine/src/interfaces/filesystem.ts`
2. **TS adapters** (6 total): `node/node-filesystem.ts`, `node/scoped-node-filesystem.ts`, `daemon/daemon-filesystem.ts`, `native/native-filesystem.ts` + `native/bindings.d.ts`, `memory/memory-filesystem.ts`, `null/null-filesystem.ts`
3. **Android FileManager**: `android/io-core/.../FileManager.kt` + `FileManagerImpl.kt`
4. **Android FileBindings**: `android/quickjs-engine/.../FileBindings.kt` (register JNI function)
5. **Android companion HTTP**: `android/companion-server/.../NettyHttpServer.kt` (add endpoint)
6. **Rust io-daemon**: `desktop/io-daemon/src/files.rs` (add endpoint + route in `main.rs`)
7. **iOS FileBindings**: `ios/JSTorrentKit/Sources/JSTorrentKit/Bindings/FileBindings.swift`
8. **Verify**: `pnpm typecheck && pnpm test` (engine), `./gradlew :app:compileDebugKotlin` (android), `cargo clippy --workspace` (desktop), `xcodebuild -scheme JSTorrent -destination 'platform=iOS Simulator,name=iPhone 16' build` (ios)

### QuickJS FFI: Boolean String Coercion Pitfall

The QuickJS JNI bridge (`setGlobalFunction` in `QuickJsContext.kt`) only supports `String?` return types. When Kotlin returns `boolean.toString()`, it produces `"true"` or `"false"` — but in JavaScript, **both are truthy** (`if ("false")` is `true`).

**Rules for the native adapter (`packages/engine/src/adapters/native/`):**
- **NEVER** use truthiness checks (`if (result)` / `if (!result)`) on values from `__jstorrent_*` functions
- **ALWAYS** use explicit comparison: `result === true || result === 'true'`
- `bindings.d.ts` declares these as `string | boolean` to flag the ambiguity
- The TCP/UDP callback dispatchers in `NativeBindings.kt` already handle this correctly with inline `=== 'true'` JS conversion

See `bindings.d.ts` header comment for full explanation.

## Git Commit Policy

**Do NOT include `Co-Authored-By` lines referencing Claude, AI, or Anthropic in commit messages. Do NOT include "Generated with Claude Code" or similar AI attribution. Commits are authored solely by the user.**

## Git Configuration and Commit Attribution

### User Identity Management

**CRITICAL**: When using Claude Code research preview (claude.ai/code), proper git commit attribution is required.

#### Before ANY git push operations:

1. **Check current git configuration**:
   ```bash
   git config user.name
   git config user.email
   ```

2. **If the email is `noreply@anthropic.com` or name is just `Claude`**:
   - **STOP** - Do not proceed with the push
   - Ask the user which identity should be used for commits
   - Configure git with the correct user details before pushing

3. **Never push commits** with these default values:
   - Name: `Claude`
   - Email: `noreply@anthropic.com`

#### Authorized Users

| Name | Email |
|------|-------|
| Kyle Graehl | kgraehl@gmail.com |
| Graehl Arts | graehlarts@gmail.com |

#### Setting Git Config

When the user confirms their identity, set git config:

```bash
git config user.name "User Name"
git config user.email "user@email.com"
```

#### Workflow

1. At the start of any session involving commits/pushes, verify git config
2. If using placeholder values, ask: "Which user are you? (Kyle Graehl or Graehl Arts?)"
3. Configure git with the appropriate credentials
4. Proceed with commits and pushes

This ensures proper commit history attribution across all work.

## Python Workflow

This project uses [uv](https://docs.astral.sh/uv/) for Python package management.

When working with Python projects:
1. Use `uv sync` to install dependencies
2. Use `uv run python script.py` to run scripts
3. Each Python project has its own `pyproject.toml` and `uv.lock`

Python projects in this repo:
- `desktop/` - Native host verification tests
- `packages/engine/integration/python/` - Engine integration tests


## Rust Editing Workflow (desktop/)

After editing Rust files in `desktop/`, run from the `desktop/` directory:

1. `cargo fmt --all` - Fix formatting (CI runs `cargo fmt --all -- --check`)
2. `cargo clippy --workspace -- -D warnings` - Run lints
3. `cargo test --workspace` - Run tests

**Note:** CI requires sidecar stubs for the Tauri build script. Clippy/tests may need them:
```bash
TRIPLE="$(rustc --print host-tuple)"
mkdir -p tauri-app/src-tauri/binaries
touch "tauri-app/src-tauri/binaries/jstorrent-host-$TRIPLE"
touch "tauri-app/src-tauri/binaries/jstorrent-io-daemon-$TRIPLE"
```

## Installing Dependencies

Use normal `pnpm install`.

```bash
pnpm install
```

If you are developing against a local `playsvideo` checkout, relink it manually after install as described above.

## TypeScript Editing Workflow

The `pnpm` scripts are for TypeScript packages (extension, engine, etc.).

After editing TypeScript files, run the following checks in order:

1. `pnpm run typecheck` - Verify type correctness
2. `pnpm run test` - Run unit tests
3. `pnpm run lint` - Check lint rules

**IMPORTANT**: Only after all edits are complete and tests pass, run as the final step:

3. `pnpm format:fix` - Fix formatting issues

Run `format:fix` last because fixing type errors or tests may introduce formatting issues that need to be cleaned up at the very end.

## Android/Kotlin Editing Workflow

After editing Kotlin/Java files in `android/`:

1. `./gradlew :app:compileDebugKotlin` - Compile Kotlin (validates types)
2. `./gradlew testDebugUnitTest` - Run unit tests
3. `./gradlew lint` - Run Android lint

### Android Emulator Management

**You CAN and SHOULD run instrumented tests on the emulator.** The emulator is easy to start and scripts handle everything automatically.

**Preamble (required before any emulator/adb commands):**
```bash
source ~/.profile && source android/scripts/android-env.sh
```

**Check if an emulator is already running:**
```bash
adb devices 2>/dev/null | grep -q 'emulator-' && echo "Running" || echo "Not running"
```

**Start the emulator (idempotent — safe to call if already running):**
```bash
emu start
```

`emu start` runs `android/scripts/emu-start.sh` which:
- Detects if an emulator is already running and exits early if so
- Starts the `jstorrent-dev` AVD in the background (headless, no audio)
- Waits for boot to complete (up to 120 seconds)
- Sets up port forwarding (7800, 7805, 7814, 7827)

**Other useful `emu` subcommands:**
```bash
emu status      # Show connected devices and port forwards
emu stop        # Stop the emulator
emu install     # Build and install the APK
emu logs        # Filtered logcat (use --js for QuickJS logs only)
emu reset       # Clear app data
```

### Running Instrumented Tests

After ensuring the emulator is running:

```bash
source ~/.profile && source android/scripts/android-env.sh
emu start   # No-op if already running

# Instrumented tests (fast, no external deps)
./gradlew connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.notPackage=com.jstorrent.app.e2e

# E2E tests (requires Python seeder)
pnpm seed-for-test &  # Auto-kills any existing seeder on port 6881
./gradlew connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.package=com.jstorrent.app.e2e

# Or use the unified test runner:
android/scripts/test.sh --integration   # Instrumented tests (requires emulator)
android/scripts/test.sh --e2e           # E2E tests (requires emulator + seeder)
android/scripts/test.sh --all           # All test suites

# Manual E2E testing
emu test-native                         # Install app, launch with test magnet
```

See `android/scripts/` for more: `emu-logs.sh`, `emu-install.sh`, `dev` commands for real devices.

### ChromeOS Companion Smoke Test

Use this to validate the full Chrome extension → Android companion → torrent download path against the local emulator:

```bash
source ~/.profile && source android/scripts/android-env.sh
DOWNLOAD_TIMEOUT_MS=720000 ./scripts/e2e-companion-smoke.sh --skip-build
```

Notes:
- This uses the local Android emulator, not physical ChromeOS hardware.
- If you changed extension code and need a fresh build, omit `--skip-build`.
- On this machine, `DOWNLOAD_TIMEOUT_MS=720000` was needed for the 100MB smoke download to complete reliably.
- First-time emulator setup: if the test connects successfully but reports zero roots, launch the add-root flow once in the emulator and approve `Download/JSTorrent` via the Android folder picker. That permission persists for later runs.
- For the larger download path, use: `FULL_DOWNLOAD=1 DOWNLOAD_TIMEOUT_MS=720000 ./scripts/e2e-companion-smoke.sh --skip-build`

## Releases

All components follow the same release pattern:
1. Update the component's `CHANGELOG.md` with a `## [VERSION]` section (required - scripts will fail without it)
2. Run the release script: `./scripts/release-{component}.sh <version>`
3. CI automatically builds and publishes artifacts when the tag is pushed

**Commit message format:** `Release {Component} v{VERSION}` (e.g., `Release Engine v1.0.1`)

### Release Pipeline Summary

| Component | Tag | CI builds | Publishing |
|-----------|-----|-----------|------------|
| **Engine (CLI)** | `engine-v{ver}` | npm package | CI auto-publishes to npm |
| **Extension** | `extension-v{ver}` | ZIP | Manual upload to Chrome Web Store |
| **Android** | `android-v{ver}` | Signed APK + AAB | Manual upload to Play Store |
| **Tauri App** | `tauri-app-v{ver}` | Signed installers (Mac/Win/Linux) | Auto-updates via updater JSON |
| **iOS** | `ios-v{ver}` | Signed IPA | AltStore PAL (EU) via Apple Notarization |
| **Website** | `website-v{ver}` | N/A | Auto-deploys on push to main |

### Version Compatibility & Release Order

The extension checks the connected backend's version against minimum requirements defined in `packages/client/src/hooks/useSystemBridge.ts` (`VERSION_REQUIREMENTS`). If the backend is too old, the extension shows "Update Required" and blocks downloads.

**Safe release order** (backends before frontends):
1. **Android** first (Play Store review takes time)
2. **Tauri App** second (auto-updates, give it a day to propagate)
3. **Extension** last (bump `VERSION_REQUIREMENTS` before releasing)

**Before every extension release, ask the user:**
- "Did this release cycle add new backend features (new endpoints, new IFileSystem methods, protocol changes) that the extension now depends on?"
- If yes: "What are the minimum Android and Tauri app versions that include these features? I'll update `VERSION_REQUIREMENTS` before releasing."
- Check that the required backend versions have already been released (git tags exist).

**Version fields checked:**
- Desktop: `desktopVersion` (Tauri app version from `tauri.conf.json`)
- Android/ChromeOS: `version` (Android app `versionName` from `build.gradle.kts`)
- Tauri (self-hosted): no check needed

### Engine (CLI) Releases

```bash
./scripts/release-engine.sh <version>
```

- Updates `packages/engine/package.json` and `packages/engine/src/version.ts`
- Creates tag: `engine-v{version}`
- CI publishes to npm as `@jstorrent/engine`
- Changelog: `packages/engine/CHANGELOG.md`

### Extension Releases

```bash
./scripts/release-extension.sh <version>
```

- Updates `extension/public/manifest.json`
- Creates tag: `extension-v{version}`
- CI creates GitHub Release with ZIP attachment
- **Manual step:** Download ZIP from GitHub Release and upload to Chrome Web Store
- Changelog: `extension/CHANGELOG.md`
- **Pre-release check:** Review `VERSION_REQUIREMENTS` in `packages/client/src/hooks/useSystemBridge.ts`. If new backend features are required, bump `minSupported` and ensure those backend versions are already released.
- **Pre-release smoke test:** Run the companion download smoke test before releasing. The release script will warn if it hasn't been run recently.
  ```bash
  ./scripts/e2e-companion-smoke.sh                       # 100MB quick test
  FULL_DOWNLOAD=1 ./scripts/e2e-companion-smoke.sh       # 1GB full test (recommended)
  ```
  Requires: Android emulator running (`emu start`). The script handles everything else (app install, companion mode, seeder, root setup, Playwright test).

### Android Releases

```bash
./scripts/release-android.sh <version>
```

- Increments `versionCode` automatically, sets `versionName`
- Creates tag: `android-v{version}`
- CI creates GitHub Release with signed APK and AAB
- Changelog: `android/CHANGELOG.md`

**Manual step:** After CI completes, upload AAB to Play Store and publish.

**Play Store bundle:** After CI completes, manually build with:
```bash
./scripts/build-playstore-bundle.sh
```
Requires upload keystore at `android/app/signing/upload.keystore`.

**Do NOT** manually edit build.gradle.kts for version bumps.

### Tauri App Releases

```bash
./scripts/release-tauri-app.sh <version>
```

- Updates `desktop/tauri-app/src-tauri/tauri.conf.json`, `desktop/tauri-app/package.json`, and `desktop/tauri-app/src-tauri/Cargo.toml`
- Creates tag: `tauri-app-v{version}`
- CI builds signed/notarized installers for macOS (aarch64 + x86_64), Windows, and Linux
- CI creates GitHub Release with updater JSON for auto-updates
- **No manual step:** Existing installs auto-update via the updater JSON endpoint
- Changelog: `desktop/tauri-app/CHANGELOG.md`

### iOS Releases (AltStore PAL)

```bash
./scripts/release-ios.sh <version>
```

- Updates `ios/project.yml` (`MARKETING_VERSION`, `CURRENT_PROJECT_VERSION` auto-incremented)
- Creates tag: `ios-v{version}`
- CI builds IPA, uploads to App Store Connect for notarization, creates draft GitHub Release
- Changelog: `ios/CHANGELOG.md`
- **Manual steps after Apple notarization approval:**
  1. Download ADP from AltStore PAL REST API
  2. Upload ADP to GitHub Release
  3. Run `scripts/ios-finalize-release.sh <version> <adp-url>`
  4. Commit updated `website/public/altstore-source.json`
  5. Undraft GitHub Release: `gh release edit ios-v<version> --draft=false`

### Website Releases

```bash
./scripts/release-website.sh <version>
```

- Creates tag: `website-v{version}` (no version file changes)
- Website auto-deploys on any push to main that touches `website/`
- The tag is for version tracking only; deployment is not gated by tags
- No changelog required

## ChromeOS Device Control

ChromeOS testbed tools (screenshot, tap, type, accessibility tree, CDP, etc.) live in the standalone `~/code/chromeos-testbed` repo. See that repo's README and `skills/SKILL.md` for setup and usage.

To use ChromeOS MCP tools in this project, create `.mcp.json` from the example:

```bash
cp .mcp.json.example .mcp.json
# Edit .mcp.json to use your actual paths (find uv path with: which uv)
```

## ChromeOS Development

When testing on ChromeOS, the extension runs on a Chromebook. The agent runs on the dev laptop.

### Build & Deploy

**Do NOT just run `pnpm build` for ChromeOS testing.** Use the deploy script:

```bash
./scripts/deploy-chromebook.sh
```

This:
1. Builds the extension locally
2. Rsyncs to Chromebook (`/mnt/chromeos/MyFiles/Downloads/crostini-shared/jstorrent-extension/`)
3. Triggers `chrome.runtime.reload()` via CDP

### Prerequisites (set up by human)

- SSH tunnel for CDP: `ssh -L 9222:127.0.0.1:9222 chromebook`
- Extension loaded once from the deploy path

### Debugging

With CDP tunnel active, use MCP tools:
- `ext_status` - Check connectivity
- `ext_get_logs` - View SW console output
- `ext_evaluate` - Inspect state

### If extension disappears

Sometimes Chrome unloads the extension. Re-load manually:
1. `chrome://extensions` on Chromebook
2. The extension may show as "errors" or be missing
3. Click "Load unpacked" again -> `Downloads/crostini-shared/jstorrent-extension/`

### Android App Deployment

Deploy the Android app to ChromeOS:

```bash
./scripts/deploy-android-chromebook.sh              # Debug build
./scripts/deploy-android-chromebook.sh release      # Release build
./scripts/deploy-android-chromebook.sh --forward    # Debug + dev server forwarding
./scripts/deploy-android-chromebook.sh release -f   # Release + dev server forwarding
```

This builds the APK locally, copies to Chromebook (at `~/code/jstorrent-monorepo/android/`), and installs via ADB.

**Dev server port forwarding (`--forward` or `-f`):**
For debug builds that load from `localhost:3000`, use `--forward` to set up:
1. SSH reverse tunnel: Your dev server → Chromebook's localhost:3000
2. ADB reverse: Chromebook localhost:3000 → Android's localhost:3000

The SSH tunnel runs in the background. To stop it: `pkill -f 'ssh.*-R 3000.*chromebook'`

**Environment variables:**
- `CHROMEBOOK_HOST` - SSH host (default: `chromebook`)
- `REMOTE_PROJECT_DIR` - Path on Chromebook (default: `/home/graehlarts/code/jstorrent-monorepo/android`)
- `DEV_SERVER_PORT` - Port to forward for dev server (default: `3000`)
- `REMOTE_ADB` - Full path to adb on Chromebook (default: `/home/graehlarts/android-sdk/platform-tools/adb`)

**ADB path on Chromebook:** `/home/graehlarts/android-sdk/platform-tools/adb`

**Troubleshooting:**
- Signature mismatch: `ssh chromebook "/home/graehlarts/android-sdk/platform-tools/adb uninstall com.jstorrent.app"` then redeploy
- ADB not available: Enable "Linux development environment" and "Android apps" in ChromeOS settings

### Real Device Testing (dev command)

The `dev` command provides unified deployment to real devices (phones and Chromebook).

**Setup:**
```bash
# Create device config (see android/scripts/devices.example)
cat >> ~/.jstorrent-devices << 'EOF'
pixel9=serial:XXXXXXXXX
motog=wifi:192.168.1.50:5555
chromebook=ssh:chromebook:~/android-sdk/platform-tools/adb
EOF

# Load shell environment (provides both emu and dev commands)
source android/scripts/android-env.sh
```

**Device config format:**
- `serial` - USB-connected device (use serial from `adb devices`)
- `wifi` - WiFi ADB device (ip:port)
- `ssh` - Remote ADB over SSH (host:adb_path)

**Commands:**
```bash
dev list                      # List configured devices and status
dev install pixel9            # Build and install debug APK
dev install pixel9 --release  # Release build
dev install pixel9 --forward  # Debug + port forwarding for dev server
dev logs pixel9               # Watch logcat
dev shell pixel9              # ADB shell
dev reset pixel9              # Clear app data
dev connect motog             # Connect WiFi ADB device
```

**Aliases:** Per-device aliases are auto-generated from your config:
```bash
dev-pixel9        # Shortcut for: dev install pixel9
dev-chromebook    # Shortcut for: dev install chromebook
```

## Viewing QuickJS JavaScript Logs

The QuickJS engine routes `console.log` to Android's logcat with tag `JSTorrent-JS`.

**Always start with this preamble** (adb requires ~/.profile for PATH, and you need to know which devices are connected):

```bash
source ~/.profile && adb devices
```

**Important:** When multiple devices are connected (e.g., emulator + physical device), you MUST specify `-s SERIAL` in **both** the outer `adb logcat` AND the inner `adb shell pidof` commands, otherwise adb fails with "more than one device/emulator".

```bash
# With single device connected:
adb logcat --pid=$(adb shell pidof com.jstorrent.app) -d -t 100

# With multiple devices - specify serial in BOTH places:
adb -s 48081FDAQ002HZ logcat --pid=$(adb -s 48081FDAQ002HZ shell pidof com.jstorrent.app) -d -t 100
adb -s emulator-5554 logcat --pid=$(adb -s emulator-5554 shell pidof com.jstorrent.app) -d -t 100

# Using emu/dev helpers (recommended - handles device selection automatically)
source android/scripts/android-env.sh
emu logs --js          # Emulator: QuickJS logs only (PID-filtered)
dev logs pixel9 --js   # Real device: QuickJS logs only (PID-filtered)
```

**Log levels:**
- `console.log()` → `Log.i` (INFO) - tag: `JSTorrent-JS`
- `console.debug()` → `Log.d` (DEBUG)
- `console.warn()` → `Log.w` (WARN)
- `console.error()` → `Log.e` (ERROR)

**Related Kotlin tags for debugging:**
- `EngineController` - Kotlin engine wrapper
- `QuickJsContext` - JS execution and job scheduling
- `TcpBindings`, `UdpBindings`, `FileBindings` - Native I/O
- `JsThread` - JS thread health monitoring (latency warnings)

## Debug Manhole (adb broadcast receiver)

The app includes a debug broadcast receiver for inspecting engine state when the UI is unresponsive or you need to diagnose issues without touching the app.

**Commands:**

```bash
# Get engine status (includes JS thread latency)
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd status -p com.jstorrent.app

# Evaluate arbitrary JavaScript in the engine
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd eval --es expr "Date.now()" -p com.jstorrent.app
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd eval --es expr "globalThis.jstorrent?.torrents?.length" -p com.jstorrent.app

# Get swarm debug info (peer connection states, errors, etc)
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd swarm -p com.jstorrent.app
# Or for a specific torrent:
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd swarm --es hash "abc123..." -p com.jstorrent.app

# Get DHT statistics
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd dht -p com.jstorrent.app

# List all torrents with details
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd torrents -p com.jstorrent.app

# List connected peers
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd peers -p com.jstorrent.app

# Set log level (debug/info/warn/error)
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd loglevel --es level debug -p com.jstorrent.app

# Show help
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd help -p com.jstorrent.app
```

**Viewing output:**

```bash
# Filter to just debug output
adb logcat -s JSTorrent-Debug

# Or include in broader logs
adb logcat | grep -i "JSTorrent-Debug"
```

**JS Thread Health Monitoring:**

The JS thread automatically logs warnings when it detects latency > 1 second:
```
JsThread: JS thread latency: 5432ms (max: 14642ms) - thread may be overloaded
```

The `status` command shows both current latency and max observed latency since engine start.

**Implementation:** `android/app/src/main/java/com/jstorrent/app/debug/DebugReceiver.kt`

## Download Speed Benchmarking

Use this to measure baseline download throughput when optimizing performance.

**Quick start:**
```bash
./scripts/benchmark-daemon-download.sh
```

**Prerequisites:**
- Configure `~/.jstorrent-devices` with:
  ```
  seeder=<ip>:6881
  benchmark_host=chromebook
  ```
- 1GB test seeder running: `pnpm seed-for-test --size 1gb`
- Android companion app running on ChromeOS
- `.env` file configured on chromebook at `~/code/jstorrent/.env`

**What it does:**
1. Reads `seeder` and `benchmark_host` from `~/.jstorrent-devices`
2. Syncs engine code to chromebook
3. Starts Node.js daemon client with `--no-session` (stateless mode)
4. Downloads 1GB test torrent via Android companion
5. Reports time and average speed
6. Cleans up (removes torrent with data)

**Typical results:** ~30 MB/s average

**Node.js daemon client flags:**
- `--no-session` - Stateless mode (MemorySessionStore), no persistence between runs
- `--help` - Show all options

**RPC endpoints used:**
- `POST /torrent/add` - Add torrent with magnet link
- `GET /torrent/{id}/status` - Get progress, speed, peers
- `POST /torrent/{id}/remove` with `{"deleteData":true}` - Remove and delete files
- `POST /shutdown` - Stop daemon

## Android SDK Setup

The Android SDK is at `~/Android/Sdk`. Gradle needs to know the SDK location via one of:
- `local.properties` with `sdk.dir` (recommended)
- `ANDROID_HOME` environment variable
- `ANDROID_SDK_ROOT` environment variable

To create `local.properties`:

```bash
echo "sdk.dir=$HOME/Android/Sdk" > android/local.properties
```

Note: `local.properties` is gitignored - each machine needs its own.

**First-time emulator setup:**
```bash
android/scripts/setup-emulator.sh
```
This creates AVDs: `jstorrent-dev`, `jstorrent-tablet`, `jstorrent-playstore`
