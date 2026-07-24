# JSTorrent for Android

The Android project ships one application with two runtime modes:

- **Standalone:** a native Jetpack Compose UI runs the shared TypeScript engine
  in-process through QuickJS-NG. Kotlin JNI bindings provide file, TCP, UDP,
  hashing, configuration, and lifecycle services.
- **ChromeOS companion:** the app exposes authenticated HTTP and WebSocket IO
  services so the Chrome extension can run the engine in its browser UI.

The mode is selected by the application and can also be changed from the UI.
Standalone Android is a shipping product, not an experimental browser wrapper.

## Project Layout

| Module | Role |
| --- | --- |
| `app` | Compose UI, application lifecycle, QuickJS bindings, persistence, playback, and search plugins |
| `quickjs-engine` | Kotlin/JNI wrapper around the QuickJS-NG submodule |
| `io-core` | Reusable socket, file, hashing, and binary protocol implementation |
| `companion-server` | Ktor HTTP/WebSocket adapter used by ChromeOS companion mode |

The authoritative SDK and application versions are in
[`app/build.gradle.kts`](app/build.gradle.kts).

## Prerequisites

- JDK 17
- Android SDK and platform tools
- Android Studio or the command-line Gradle workflow
- initialized QuickJS-NG submodule

From the repository root:

```bash
git submodule update --init android/quickjs-engine/src/main/cpp/quickjs-ng
```

If the local SDK path is not supplied by `ANDROID_HOME`, create an untracked
`android/local.properties` containing:

```text
sdk.dir=/absolute/path/to/Android/Sdk
```

## Build and Install

Run Gradle commands from `android/`:

```bash
./gradlew :app:assembleDebug
./gradlew :app:installDebug
./gradlew :app:assembleRelease
./gradlew :app:bundleRelease
```

Debug APK:
`app/build/outputs/apk/debug/app-debug.apk`

The release build requires the signing properties used by CI or the local
Play Store bundle helper.

## Tests

The test wrapper separates device-free, instrumented, and end-to-end suites:

```bash
./scripts/test.sh
./scripts/test.sh --integration
./scripts/test.sh --e2e --start-seeder
./scripts/test.sh --all --start-seeder
```

Use `--device SERIAL` to select a non-emulator device. Direct Gradle gates used
during normal Kotlin work are:

```bash
./gradlew :app:compileDebugKotlin
./gradlew testDebugUnitTest
```

## Emulator and Device Helpers

One-time command-line emulator setup:

```bash
./scripts/setup-emulator.sh
```

Load the helper commands:

```bash
source scripts/android-env.sh
emu start
emu install
emu logs
```

Real-device aliases use `~/.jstorrent-devices`; copy the format from
[`scripts/devices.example`](scripts/devices.example), then use `dev list`,
`dev install NAME`, and `dev logs NAME`.

ChromeOS-specific deployment is handled from the repository root:

```bash
./scripts/deploy-android-chromebook.sh
```

This builds locally and delegates ARCVM installation and authorization to
`~/code/chromeos-testbed`; Crostini is not required. See the
[ChromeOS hardware-testing topic](../docs/topics/chromeos-hardware-testing.md)
for device setup, forwarding, and recovery.

## Logging and Debugging

Useful logcat tags include:

- `JSTorrent-JS` for JavaScript console output
- `EngineController` and `JsThread` for engine lifecycle and latency
- `TcpBindings`, `UdpBindings`, and `FileBindings` for native IO
- `JSTorrent-Debug` for debug-broadcast output

With one device connected:

```bash
adb logcat --pid=$(adb shell pidof com.jstorrent.app)
```

The debug build exposes an ADB broadcast receiver:

```bash
adb shell am broadcast \
  -a com.jstorrent.DEBUG \
  --es cmd status \
  -p com.jstorrent.app
```

Other supported commands include `torrents`, `peers`, `swarm`, `dht`, `eval`,
`loglevel`, and `help`.

## Architecture Entry Points

- [`NativeStandaloneActivity.kt`](app/src/main/java/com/jstorrent/app/NativeStandaloneActivity.kt):
  standalone Compose application entry
- [`JSTorrentApplication.kt`](app/src/main/java/com/jstorrent/app/JSTorrentApplication.kt):
  process-level service and engine ownership
- [`EngineServiceRepository.kt`](app/src/main/java/com/jstorrent/app/viewmodel/EngineServiceRepository.kt):
  UI access to the engine service
- [`IoDaemonService.kt`](app/src/main/java/com/jstorrent/app/service/IoDaemonService.kt):
  companion foreground service
- [`CompanionServerDepsImpl.kt`](app/src/main/java/com/jstorrent/app/CompanionServerDepsImpl.kt):
  application wiring for companion mode
- [`AndroidSearchPluginSandboxHost.kt`](app/src/main/java/com/jstorrent/app/search/AndroidSearchPluginSandboxHost.kt):
  local WebView search-plugin sandbox

The normative companion protocol is
[`docs/contracts/io-daemon-contract.md`](../docs/contracts/io-daemon-contract.md).
Platform sandbox and search-plugin boundaries are tracked in
[`docs/topics/sandbox-and-search-plugin-trust-boundaries.md`](../docs/topics/sandbox-and-search-plugin-trust-boundaries.md).

## QuickJS Native-Bridge Invariant

The JNI `setGlobalFunction` bridge can return booleans as the strings `"true"`
and `"false"`. JavaScript callers of `__jstorrent_*` functions must not use
truthiness checks on those values because `"false"` is truthy. Compare
explicitly:

```ts
result === true || result === 'true'
```

The native binding declarations intentionally use `string | boolean` where
this ambiguity exists.

## Release

Android releases are created with `./scripts/release-android.sh <version>`.
Read the [release topic](../docs/topics/releases.md) before running it; the
script commits, pushes, and tags the release.
