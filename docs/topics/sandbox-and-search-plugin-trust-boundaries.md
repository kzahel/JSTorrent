# Sandbox and Search Plugin Trust Boundaries

Topic: `sandbox-and-search-plugin-trust-boundaries`

Status: **living trust-boundary record; implementation details were last
reconciled on 2026-03-13, and policy claims must be revalidated before a Google
Play submission**.

This document summarizes the sandbox and trust boundaries for JSTorrent across the current shipping platforms:

- Chrome extension
- Tauri desktop app
- Android standalone app
- Android companion mode on ChromeOS
- Search plugin execution on browser/Tauri/Android

It also includes the current Google Play review implications for the Android search plugin architecture.

## Why this exists

The search plugin feature introduces user-installed JavaScript code that is fetched from a remote URL and interpreted at runtime. That is materially different from the rest of the app and needs a precise description:

- what code is remote
- where it runs
- what app APIs it can and cannot reach
- which network paths it can use
- what to tell Google Play reviewers and users

Do not describe this as "arbitrary remote code execution" without qualification. The current design is narrower:

- plugin code is JavaScript only
- plugin code runs inside a dedicated sandbox runtime
- plugin code has no direct access to app storage or Android APIs
- plugin network access is mediated by host code and constrained by the plugin manifest host list

That said, plugin code is still remote, user-installable, and updateable, so it deserves separate policy and security review.

## Platform Sandbox Matrix

| Platform | Main UI/runtime | Engine location | Native boundary | Key sandbox/trust boundary |
| --- | --- | --- | --- | --- |
| Chrome extension | MV3 extension page (`extension/src/ui/app.html`) | TypeScript engine in extension UI page | Native Messaging to desktop host, or paired Android companion on ChromeOS | Browser extension sandbox plus Chrome permission model |
| Tauri desktop app | Tauri webview with the same client assets | TypeScript engine in the webview | Tauri commands/plugins and sidecar binaries (`jstorrent-host`, `jstorrent-io-daemon`) | Tauri capability model plus native host/daemon boundary |
| Android standalone | Native Compose UI | QuickJS engine in-process via JNI | Kotlin bindings for file/network/hash operations | No OS-level sandbox between app UI and engine; trust boundary is the JNI/native binding surface |
| Android companion mode | Native Compose UI + foreground services | No local torrent engine for extension mode; companion exposes I/O services | Local HTTP + WebSocket server for paired Chrome extension | Pairing token and local service boundary |
| Search plugin sandbox | Dedicated iframe/WebView runtime | Interpreted JavaScript plugin module | Host-mediated fetch bridge only | Separate search plugin sandbox inside browser/Tauri/Android |

## Chrome Extension

Current shape:

- The extension UI runs in a regular extension page, not in the service worker.
- The extension requires either the desktop native host or the Android companion for filesystem/socket I/O.
- The search plugin runtime is split out into a dedicated extension sandbox page:
  - [`extension/public/manifest.json`](../../extension/public/manifest.json)
  - [`packages/client/search-plugin-sandbox/search-plugin-sandbox.html`](../../packages/client/search-plugin-sandbox/search-plugin-sandbox.html)
  - [`packages/client/search-plugin-sandbox/search-plugin-sandbox.js`](../../packages/client/search-plugin-sandbox/search-plugin-sandbox.js)

Important boundaries:

- `manifest.json` declares `search-plugin-sandbox.html` as a Chrome extension sandbox page.
- The iframe host sets `sandbox="allow-scripts"` for the plugin iframe path:
  - [`packages/client/src/search/iframe-search-plugin-sandbox-host.ts`](../../packages/client/src/search/iframe-search-plugin-sandbox-host.ts)
- Plugin source is evaluated inside the sandbox page with `new Function(...)`.
- Plugin code does not get direct Chrome extension APIs.
- Plugin code does not perform direct network requests. Instead, it asks the host for `fetchText` / `fetchJson`.
- Host fetches are checked against the plugin manifest `hosts` allowlist:
  - [`packages/client/src/search/plugin-utils.ts`](../../packages/client/src/search/plugin-utils.ts)

Practical summary:

- The extension sandbox is meaningful for the search plugin feature.
- The plugin can parse HTML, make mediated HTTP(S) requests, emit results, and log.
- The plugin cannot directly touch local files, native messaging, or extension APIs.

## Tauri Desktop App

Current shape:

- The desktop app bundles the same client assets as the extension UI.
- The torrent engine runs in the Tauri webview.
- Native capabilities are provided by Tauri plugins and sidecar binaries:
  - [`desktop/tauri-app/src-tauri/tauri.conf.json`](../../desktop/tauri-app/src-tauri/tauri.conf.json)
  - [`desktop/tauri-app/src-tauri/capabilities/default.json`](../../desktop/tauri-app/src-tauri/capabilities/default.json)

Important boundaries:

- The main app uses Tauri's native boundary for privileged actions.
- The default capability grants a broad set of permissions, including shell execution/spawn for app-managed native flows.
- The main Tauri app CSP is currently `null`, so the desktop story is not "tight web CSP plus no native escape." It depends more on Tauri capability control and the app's own boundaries.
- The search plugin feature still uses the client-side iframe sandbox host when available:
  - [`packages/client/src/search/iframe-search-plugin-sandbox-host.ts`](../../packages/client/src/search/iframe-search-plugin-sandbox-host.ts)

Practical summary:

- The search plugin runtime is still isolated from direct app APIs, but the overall desktop app is not as strictly web-sandboxed as the extension.
- The strongest boundary on desktop is the split between the webview and the native sidecars/host, not the browser security model.

## Android Standalone App

Current shape:

- The Android app has a native Compose UI.
- The torrent engine runs in-process inside QuickJS with JNI bindings for filesystem/network/hash operations.
- The app requests standard torrent-related permissions such as network access and foreground service.
- The manifest currently enables cleartext traffic globally:
  - [`android/app/src/main/AndroidManifest.xml`](../../android/app/src/main/AndroidManifest.xml)

Important boundaries:

- There is no OS-level sandbox between the app's own UI process and the QuickJS engine. This is one app process.
- The trust boundary is therefore the native binding surface:
  - which JS-callable native methods exist
  - which arguments they accept
  - what filesystem/network scope they expose

Practical summary:

- "QuickJS sandbox" should not be described as a full security sandbox for arbitrary hostile app code.
- It is a runtime isolation layer inside the app, not a separate Android app sandbox.

## Android Companion Mode

Current shape:

- On ChromeOS, the Android app can run in companion mode for the browser extension.
- The companion exposes local HTTP and WebSocket services to the paired extension.
- Access is gated by pairing and tokens rather than by Android WebView isolation.

Practical summary:

- This is a transport/security boundary, not a code-execution sandbox.
- The important controls here are pairing, token validation, and limiting the service surface to the required daemon operations.

## Search Plugin Sandbox

This is the feature most likely to matter for Play review.

### Browser and Tauri

The browser/Tauri implementation uses:

- a dedicated sandbox document:
  - [`packages/client/search-plugin-sandbox/search-plugin-sandbox.html`](../../packages/client/search-plugin-sandbox/search-plugin-sandbox.html)
- a dedicated sandbox runtime:
  - [`packages/client/search-plugin-sandbox/search-plugin-sandbox.js`](../../packages/client/search-plugin-sandbox/search-plugin-sandbox.js)
- an iframe host:
  - [`packages/client/src/search/iframe-search-plugin-sandbox-host.ts`](../../packages/client/src/search/iframe-search-plugin-sandbox-host.ts)

Execution model:

1. Plugin source is fetched by the host.
2. Source is inspected for `manifest` and `search(...)`.
3. Source is evaluated inside the sandbox document.
4. The plugin gets a small `ctx` object:
   - `encode`
   - `fetchText`
   - `fetchJson`
   - `parseHtml`
   - `emitResult`
   - `log`
5. All plugin network requests flow back through host-mediated fetch.
6. Host fetches are restricted to the plugin's declared `hosts`.

### Android

The Android implementation uses:

- a dedicated local asset WebView host:
  - [`android/app/src/main/java/com/jstorrent/app/search/AndroidSearchPluginSandboxHost.kt`](../../android/app/src/main/java/com/jstorrent/app/search/AndroidSearchPluginSandboxHost.kt)
- a fetch mediator:
  - [`android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt`](../../android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt)
- manifest and fetch normalization helpers:
  - [`android/app/src/main/java/com/jstorrent/app/search/SearchPluginUtils.kt`](../../android/app/src/main/java/com/jstorrent/app/search/SearchPluginUtils.kt)

The Android host currently does all of the following:

- loads only a local sandbox asset URL
- enables JavaScript
- disables DOM storage
- disables file access
- disables content access
- blocks navigation outside the sandbox asset path
- blocks non-asset requests in `shouldInterceptRequest`
- injects a very small JS bridge shim after page load
- removes the JS interface and destroys the WebView on dispose

Plugin network access is still mediated by host Kotlin code. The plugin does not directly navigate the WebView to remote pages.

### What the plugin can do

- Execute JavaScript logic
- Parse remote HTML/JSON responses
- Make host-mediated HTTP requests
- Return search results, including magnet links and torrent URLs

### What the plugin cannot do directly

- Use Android APIs
- Use Chrome extension APIs
- Read app files
- Access SAF roots
- Use native messaging
- Open arbitrary WebView pages
- Fetch arbitrary hosts outside its manifest allowlist

## Current Gaps and Review Risks

These are the main caveats to document honestly.

### 1. Remote interpreted code is real

The code is not bundled in the APK when a user installs a plugin from URL. It is fetched later and interpreted at runtime.

That means:

- the feature should be treated as user-installed interpreted code
- the install/update path should remain explicit and user-initiated
- reviewer-facing language should be careful and precise

### 2. Android plugin fetch currently allows both HTTP and HTTPS

The Android plugin fetch mediator accepts `http` and `https` URLs:

- [`android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt`](../../android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt)

This is the sharpest Play-review risk in the current implementation. Even though the WebView itself blocks direct remote navigation, the overall feature still allows a plugin to fetch its source data over cleartext HTTP through the host mediator if the plugin manifest declares such a host.

### 3. Android app manifest allows cleartext traffic globally

The manifest sets `android:usesCleartextTraffic="true"`:

- [`android/app/src/main/AndroidManifest.xml`](../../android/app/src/main/AndroidManifest.xml)

That may be justified for some torrent/network paths, but it weakens the security story for the search plugin feature unless the plugin path is separately constrained.

### 4. Tauri app CSP is currently disabled

The desktop app's Tauri config sets:

- `"csp": null`

That does not break the search plugin sandbox design by itself, but it means the desktop app should not be described as relying on a strict web CSP.

## Google Play Implications

As of March 13, 2026, I did not find an official Google Play Help Center page describing a dedicated Play Console declaration question specifically for "interpreted remote JavaScript search plugins."

That is an inference from the currently available official docs, not a guarantee that a Play Console form or reviewer questionnaire cannot change later.

The official docs that look most relevant are:

- Google Play Developer Program Policies, "Device and Network Abuse":
  - https://support.google.com/googleplay/android-developer/answer/9888379
- Google Play Help, "Remediation for WebView code injection":
  - https://support.google.com/faqs/answer/9095419
- Google Play policy update note on "Behavior Transparency":
  - https://support.google.com/googleplay/android-developer/thread/333341051/changes-to-google-play-s-developer-program-policies-and-updates-to-the-behavior-transparency-policy

### Practical interpretation

- There does not appear to be a simple "does your app execute remote code?" listing checkbox in the official help docs I found.
- The bigger issue is whether the feature is transparent, user-initiated, and sufficiently constrained.
- Reviewers are likely to care more about:
  - whether this is hidden or dormant functionality
  - whether remote code can escape the sandbox
  - whether the app loads untrusted content into a privileged WebView
  - whether cleartext traffic is involved

## Recommended Play Store and Review Changes

### 1. Update the store description

The listing should mention the feature plainly. Suggested language:

"Includes optional installable search provider plugins. Plugins run in a restricted sandbox and are limited to their declared network sources."

That is better than either extreme:

- too vague: "Search plugins"
- too alarming/inaccurate: "Executes remote code"

### 2. Update "What's new" when this ships

If the Play listing already has users who do not expect installable search providers, the release notes should mention it explicitly for the release that introduces or substantially expands the feature.

### 3. Add or preserve in-app install disclosure

The plugin install/update flow should show:

- source URL
- plugin name
- declared hosts
- whether this is an install or update

If Play review ever questions the feature, explicit user-driven installation is part of the defense.

### 4. Keep a reviewer note ready

Suggested reviewer note:

"JSTorrent supports optional user-installed search provider plugins. These plugins are JavaScript modules interpreted inside a dedicated sandbox runtime. They do not receive direct access to Android APIs, app storage, contacts, sensors, or app permissions. Plugin network requests are mediated by app code and restricted to hosts declared in each plugin manifest. Plugin installation and updates are user-initiated."

### 5. Strongly consider HTTPS-only plugin traffic

If you want the cleanest Play-review posture, the next hardening step is:

- require HTTPS for plugin source download
- require HTTPS for plugin fetches
- keep any unavoidable cleartext traffic limited to non-plugin torrent paths

That change is more important than any listing wording tweak.

## Recommended Next Technical Steps

If the goal is to reduce Play review risk, this is the order that makes sense:

1. Restrict search plugin source and fetch traffic to HTTPS only.
2. Document why `usesCleartextTraffic="true"` is still needed for non-plugin paths, or remove/narrow it if possible.
3. Make plugin install/update disclosure explicit in the Android UI if it is not already.
4. Update the Play listing text to mention installable search providers.
5. Keep the reviewer note above in the Play Console submission notes.

## Bottom Line

For Play, the current issue is probably not "you must answer a special remote code question in the listing."

The more realistic issue is:

- you now have user-installed interpreted code
- the current sandbox story is decent but needs to be documented clearly
- the weakest point is cleartext/plugin fetch policy, not store copy

If a reviewer looks closely, the strongest answer is a combination of:

- accurate store wording
- explicit in-app disclosure
- a short reviewer note
- tighter technical constraints on plugin traffic
