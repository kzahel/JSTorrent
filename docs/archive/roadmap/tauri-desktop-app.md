# Tauri Desktop App

**Status:** Roadmap / Future
**Priority:** Nice-to-have
**Audience:** Desktop users who don't use Chrome or prefer a standalone app

---

## Motivation

The Chrome extension + native host is the primary desktop experience and works well. But some desktop users are left out:

- **Firefox, Safari, Brave users** — no extension path at all today
- **Users who dislike extensions** — want a native app experience
- **Minimal install preference** — one app instead of extension + native host

The Tauri app is a **supplemental** distribution that serves these users. It does not replace the extension — Chrome users should continue using the extension for the tightest integration.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Tauri Desktop App                     │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │          WebView (WebKit / WebView2)             │   │
│  │                                                   │   │
│  │   ┌─────────────────┐   ┌─────────────────────┐  │   │
│  │   │    React UI     │   │  @jstorrent/engine  │  │   │
│  │   │   (existing)    │◄──►  (TypeScript)       │  │   │
│  │   └─────────────────┘   └─────────────────────┘  │   │
│  │                                │                  │   │
│  └────────────────────────────────┼──────────────────┘   │
│                                   │ I/O bridge           │
│  ┌────────────────────────────────▼──────────────────┐   │
│  │               Rust Backend                         │   │
│  │                                                    │   │
│  │  ┌─────────────┐  ┌───────────┐  ┌────────────┐   │   │
│  │  │   Sockets   │  │   Files   │  │  Hashing   │   │   │
│  │  │ (TCP/UDP)   │  │  (read/   │  │  (SHA1)    │   │   │
│  │  │             │  │   write)  │  │            │   │   │
│  │  └─────────────┘  └───────────┘  └────────────┘   │   │
│  └────────────────────────────────────────────────────┘   │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │  Tauri Shell: tray icon, auto-update, deep links   │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

### Key Insight: No Node.js, No QuickJS

The Tauri webview **is** a JavaScript runtime (WebKit on Mac, WebView2 on Windows/Linux). The engine runs directly in the webview, just like it runs in the Chrome extension's service worker today.

The webview gives us V8/JSC for free.

### I/O Bridge: Sidecar

**Decision: Option A (sidecar).** The engine talks to io-daemon via the existing daemon bridge adapter (WebSocket/HTTP to localhost). This is simpler than the extension path — no native host middleman, since io-daemon already has a `/control` HTTP interface.

#### Option A: Sidecar (reuse existing HTTP/WS transport)

Bundle io-daemon as a Tauri sidecar process. The engine in the webview connects via the existing daemon bridge adapter (WebSocket/HTTP to localhost), identical to how the extension works today.

**Pros:**
- Minimal new code — reuse the existing daemon bridge adapter as-is
- Proven, tested I/O path shared with the extension
- io-daemon stays a standalone binary, no refactoring

**Cons:**
- Two processes (Tauri app + sidecar), two binaries to sign
- Localhost network hop for all I/O
- Port allocation, sidecar visible in task manager
- Sidecar lifecycle management

#### Option B: In-process invoke()

Refactor io-daemon's I/O core into a library crate. Expose it as `#[tauri::command]` handlers. The engine calls Rust directly via Tauri's `invoke()` IPC.

```typescript
import { invoke } from '@tauri-apps/api/core';

class TauriSocketFactory implements ISocketFactory {
  async createTcpSocket(): Promise<ITcpSocket> {
    const id = await invoke('tcp_connect', { host, port });
    return new TauriTcpSocket(id);
  }
}
```

**Pros:**
- Single process, clean UX
- Lower latency (in-process IPC vs network), matters for high-throughput piece data
- No port allocation, no sidecar management
- Analogous to Android's JNI approach — proven pattern

**Cons:**
- New Tauri adapter needed (`TauriSocketFactory`, `TauriFileSystem`, etc.)
- Requires extracting io-daemon I/O into a shared library crate
- More upfront work, additional adapter to maintain

Option B remains a possible future optimization if profiling shows the localhost hop matters for throughput.

---

### Window Lifecycle & Background Downloads

**Decision: Engine lives in the main (only) window (Option A).** The engine and UI share the same webview JS context — same as the Chrome extension. When the user closes the window, we hide it instead of destroying it, preserving all JS state (engine, WebSocket connections, timers).

Alternative considered: a dedicated hidden "engine" webview with a separate "UI" webview communicating via Tauri events. Rejected — adds IPC complexity for no real benefit. If the UI webview ever needs to be recreated (crash recovery), session persistence handles restoring engine state.

**Key mechanisms:**

1. **Hide on close** — Intercept `WindowEvent::CloseRequested`, call `window.hide()` + `api.prevent_close()`. The webview stays alive.

2. **Prevent app exit** — Switch from `Builder::run()` to `Builder::build()` + `app.run()` with an `ExitRequested` handler that calls `api.prevent_exit()` when `code.is_none()` (triggered by last window hiding, not explicit quit).

3. **System tray** — Required so users can re-show the window and actually quit. Tray menu: Show / Quit (where Quit calls `app.exit(0)`). Left-click on tray icon shows the window.

4. **Background throttling** — macOS aggressively throttles hidden webviews. Set `"backgroundThrottling": "disabled"` in `tauri.conf.json` window config. This is macOS 14+ only; Linux and Windows don't aggressively throttle, so they're fine without it.

5. **Sidecar lifecycle** — Hold onto the `CommandChild` handle from sidecar spawn. This enables future idle shutdown (kill io-daemon when no active torrents, restart on demand).

---

## What Tauri Provides

| Feature | Benefit |
|---------|---------|
| Cross-platform webview | Same UI code works everywhere |
| Auto-updater | GitHub Releases integration, signature verification |
| Code signing | Unified workflow, just set env vars |
| System tray | Status icon, quick actions |
| Deep links | `jstorrent://` protocol handler |
| Native dialogs | File picker, notifications |
| Small binary | ~10-20MB (no Electron bloat) |

---

## Distribution

| Platform | Installer | Signing |
|----------|-----------|---------|
| macOS | `.dmg` | Apple Developer ($99/yr) - already have |
| Windows | `.msi` / `.exe` | Azure Trusted Signing - scaffolding exists |
| Linux | `.deb`, `.rpm`, `.AppImage` | None needed |

---

## What We Reuse

| Component | Source | Notes |
|-----------|--------|-------|
| React UI | `packages/ui/` | Same UI, different host |
| Engine | `packages/engine/` | Same adapter (sidecar) or new adapter (invoke) |
| Rust I/O | `desktop/io-daemon/` | Sidecar as-is, or extract into shared crate |
| Signing infra | `desktop/windows_signing/` | Same Azure setup |

---

## What's New

1. **Tauri app shell** in `packages/desktop-app/` (new)
   - `src-tauri/` - Rust backend, sidecar config or I/O commands
   - Tray icon, auto-update config
   - Deep link registration

2. **CI workflow** for Tauri builds
   - Matrix build for macOS (arm64 + x86_64), Windows, Linux
   - Code signing integrated
   - GitHub Releases upload

3. **If Option B:** Tauri adapter in `packages/engine/src/adapters/tauri/`
   - `TauriSocketFactory`, `TauriFileSystem`, `TauriSessionStore`, `TauriHasher`

---

## Relationship to Other Desktop Paths

The Tauri app is **supplemental**, not a replacement:

- **Chrome extension + native host** — remains the primary desktop experience for Chrome users
- **jstorrent.com + native host** — serves other Chromium-based browsers (Edge, Brave)
- **Tauri app** — serves users with no Chrome at all (Firefox, Safari) or who prefer standalone apps

All three coexist. Users choose the path that fits their setup.

---

## Windows Signing

Windows code signing for the current native host + io-daemon has never been fully working. macOS signing/notarization works fine.

Tauri has built-in Windows code signing support — configure certificate env vars and `tauri build` handles signing the binary + installer. This is battle-tested by the Tauri community with well-documented CI recipes, including Azure Trusted Signing.

This is a pragmatic reason to prioritize the Tauri app for Windows specifically. Rather than continuing to fight custom signing integration for the native host binaries, the Tauri app could become the primary Windows distribution path.

---

## Open Questions

- ~~**invoke() vs sidecar?**~~ **Decided: sidecar.** See I/O Bridge section above.
- ~~**WebView lifecycle?**~~ **Decided: hide on close, single webview (engine + UI together).** See Window Lifecycle section above.
- **Universal binary (macOS)?** Ship fat binary or separate arm64/x86_64?
- **Auto-update UX?** Prompt on tray click, or background download + restart?
- **WebView API parity?** Does the webview have all APIs the engine uses? (Likely yes, but verify)
- **Magnet handling?** Tauri can register URL protocols. Needs wiring.
- **webkit2gtk on Linux?** Runtime dependency — not always present on minimal distros.
- **Sidecar idle shutdown?** Kill io-daemon when no active torrents to save resources? Restart on demand via stored `CommandChild` handle. Nice-to-have.

---

## Why Not Electron?

- 150MB+ overhead vs Tauri's ~20MB
- Chromium update burden
- Tauri uses system webview (WebKit on Mac is always up-to-date)

---

## References

- [Tauri](https://tauri.app/) - Rust-based app framework
- [Tauri sidecar](https://v2.tauri.app/develop/sidecar/) - bundling external binaries
- [yepanywhere desktop roadmap](https://github.com/kgraehl/yepanywhere/blob/main/docs/roadmap/desktop-app.md) - similar architecture

---

## Estimated Effort

**Option A (sidecar):**
1. Tauri app shell + sidecar config - 1-2 weeks
2. CI/CD - 1 week
3. Testing - 1 week
Total: ~3-4 weeks

**Option B (invoke):**
1. Extract io-daemon into shared crate - 1 week
2. Tauri adapter + commands - 1-2 weeks
3. Tauri app shell - 1 week
4. CI/CD - 1 week
5. Testing - 1 week
Total: ~5-6 weeks

Low priority since the extension setup covers most desktop users, but a nice offering for broader reach.
