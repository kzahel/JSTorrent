# Standalone Daemon Mode

**Status:** Design / brainstorming
**Date:** 2026-03-03

## Overview

Upgrade the io-daemon's `--standalone` mode from a Crostini-specific workaround into a first-class deployment option. A single binary that any user can download and run, providing full JSTorrent functionality via a browser pointed at `localhost` or `jstorrent.com`.

## Product vision

```
┌────────────────────────────────────────────────────────────────────────┐
│  New deployment option                                                 │
│                                                                        │
│  User downloads io-daemon binary                                       │
│       ↓                                                                │
│  Runs: ./jstorrent-io-daemon --standalone                              │
│       ↓                                                                │
│  Opens browser → jstorrent.com  (or localhost dev server)              │
│       ↓                                                                │
│  Website detects local daemon on localhost:7800                         │
│       ↓                                                                │
│  Full torrent client — no extension, no app store, no install          │
│                                                                        │
│  Why this works: HTTPS pages can access localhost (secure context).     │
│  fetch('http://localhost:7800/...') and ws://localhost:7800/io          │
│  are allowed from https://jstorrent.com.                               │
└────────────────────────────────────────────────────────────────────────┘
```

### Deployment matrix (updated)

| Platform | Setup | Engine runs in | I/O backend |
|----------|-------|---------------|-------------|
| Mac/Win/Linux (extension) | Extension + Desktop app | Extension UI page | Rust io-daemon (via native host) |
| Mac/Win/Linux (standalone) | **io-daemon binary only** | **Website or local dev** | **Rust io-daemon** |
| Mac/Win/Linux (Tauri) | Desktop app alone | Tauri webview | Rust io-daemon |
| ChromeOS (extension + Android) | Extension + Android app | Extension UI page | Android companion |
| ChromeOS (Crostini) | Extension + io-daemon | Extension UI page | Rust io-daemon (standalone) |
| ChromeOS (Crostini, no ext) | **io-daemon binary only** | **Website** | **Rust io-daemon** |

## Architecture

Single process. No native host needed. The io-daemon in standalone mode handles everything: control, I/O, config persistence, self-update.

```
Browser (extension or website)
    │
    ├── HTTP REST ──→  io-daemon (standalone)
    │                    ├── /status, /health
    │                    ├── /roots (GET/POST)
    │                    ├── /browse?path=...
    │                    ├── /self-update
    │                    └── /read, /write, /ops/...
    │
    └── WebSocket ──→  /io (control + I/O opcodes, single connection)
                         ├── TCP/UDP multiplexing
                         ├── Auth handshake
                         └── Control messages (KV, etc.)
```

Config persisted to `~/.config/jstorrent-standalone/config.json`.

## Features

### 1. Self-update (user-initiated)

The extension/website already knows the daemon's version from `/status`. When a newer version exists (compare against latest GitHub Release), show an "Update Available" prompt.

**Daemon side:**
- `POST /self-update` endpoint (standalone mode only)
- Downloads new binary from GitHub Releases
- Atomically replaces itself (`rename()` over its own path)
- Detects if managed by systemd: `std::env::var("INVOCATION_ID").is_ok()`
  - **systemd**: exit(0), systemd `Restart=on-failure` relaunches new version
  - **manual run**: exit with message to restart, or `exec()` self with same args
- The daemon must NOT auto-update silently — only when the user triggers it

**Client side:**
- Compare `/status` version against latest GitHub Release
- Show update prompt with changelog (match existing update UX for Android/desktop — see `useSystemBridge.ts` VERSION_REQUIREMENTS pattern)
- Call `POST /self-update` when user clicks "Update"
- Handle brief disconnection during restart, reconnect automatically

### 2. Metrics check-in (24h cadence)

Background tokio task in standalone mode.

- POST to telemetry endpoint every 24 hours
- Payload: version, platform (`crostini` / `standalone`), arch, uptime, paired (bool)
- Fire-and-forget — failures silently ignored
- Match cadence/style of Tauri desktop app's headless updater (`desktop/tauri-app/src-tauri/src/headless_updater.rs`)
- Investigate whether we can reuse the same server endpoint

### 3. Download roots management

**Daemon side:**
- `GET /roots` — already exists, return configured roots
- `POST /roots` — add/remove download roots, persist to config file
- `GET /browse?path=/some/path` — directory listing for the folder browser
  - Returns: `{ entries: [{ name, type: "dir"|"file" }], writable: bool }`
  - Filter to directories only (or optionally show files)
  - No external dependencies (no zenity, no GTK — just `readdir`)
- Default root: `~/Downloads` (or `--download-root` flag)
- Persist roots to `~/.config/jstorrent-standalone/config.json`
- Install script (`website/public/install-crostini.sh`) should support `--download-root` flag, baked into systemd service file
- Re-running install script preserves config (reads from `install.conf`)

**Client side:**
- Folder browser modal (inline in the app, not a popup window)
  - Breadcrumb path bar + folder list
  - Navigate into directories, click "Select this folder"
  - Show free space, flag unwritable paths (nice-to-have)
- Text input fallback — user can also just type a path directly
- Calls `POST /roots` to register selected folder

### 4. Install script updates

Update `website/public/install-crostini.sh`:
- Accept `--download-root /path` flag
- Persist to `~/.config/jstorrent-standalone/install.conf`
- Re-running script reads existing config to preserve custom settings
- Self-update endpoint also preserves config (it only replaces the binary, not the config)

## Design decisions

### Why single process (not native host + io-daemon)?

The two-process split on desktop exists because Chrome native messaging requires a host process that communicates via stdin/stdout. On Crostini/standalone there's no native messaging, so there's no reason for the split. The io-daemon already handles both control and I/O opcodes on one WebSocket.

Systemd handles lifecycle (restart on crash). The extension handles reconnection (probes ports, re-establishes WebSocket). A supervisor process would be redundant.

### Why build our own folder browser instead of using OS file picker?

- Zero dependencies — no zenity, GTK, or desktop environment needed
- Works on headless Crostini and any Linux setup
- Consistent UX — looks like the rest of JSTorrent, not a random GTK dialog
- We control the experience (show free space, filter paths, remember recent)
- The daemon already has filesystem access; `GET /browse` is trivial to implement
- Naturally discovers USB/removable drive paths — user just browses to `/mnt/chromeos/removable/`

### Why website-as-client works

HTTPS pages can access `http://localhost` and `ws://localhost` because localhost is a secure context. This means `jstorrent.com` can talk directly to a local io-daemon. The client code already knows how to connect to the daemon over HTTP + WebSocket — it just needs to be served from the website in addition to being bundled in the extension.

### Metadata / session storage

The engine needs to persist torrent metadata (torrent list, per-torrent state, bitfields, cached peers, .torrent files, info dicts, DHT state) across browser sessions. Where this data lives depends on whether the extension is installed:

**Website with extension detected:**
- Delegate to the extension's storage via `ExternalChromeStorageSessionStore` (`chrome.runtime.sendMessage(extensionId, ...)`)
- The extension decides the actual backend: `chrome.storage.local`, Android SQLite (via companion), or desktop SQLite (via Tauri native host)
- Website stays stateless — the extension owns persistence

**Website without extension (standalone daemon mode):**
- Use `IndexedDbSessionStore` directly in the browser
- Already implemented in `packages/engine/src/adapters/browser/indexeddb-session-store.ts` (currently unused)
- Async, ~50 MB+ limit, no main thread blocking
- Data lives in the browser profile's IndexedDB — survives page reloads, cleared if user clears site data

**The daemon itself stores no session metadata.** It is purely an I/O layer. All metadata persistence is the browser/engine's responsibility. The daemon only persists its own config (roots, auth token) to `~/.config/jstorrent-standalone/config.json`.

## Open questions / things to test

- [ ] **ChromeOS removable drive paths**: What does `/mnt/chromeos/removable/` look like when a USB drive is shared with Crostini? Need to test on actual hardware.
- [ ] **Metrics endpoint**: Does the Tauri headless updater endpoint already exist and accept arbitrary platform strings? Can we reuse it?
- [ ] **CORS**: Will `fetch('http://localhost:7800')` from `https://jstorrent.com` need CORS headers? Almost certainly yes — the daemon needs to send `Access-Control-Allow-Origin` for the website origin. Check what headers are already set.
- [ ] **Website client build**: How much work to serve the client UI from jstorrent.com? Is it the same bundle as the extension, or does it need a separate entry point?
- [ ] **Mixed content warnings**: Verify that browsers don't block `http://localhost` from HTTPS pages in practice (should be fine per spec, but worth confirming across Chrome/Firefox/Safari).
- [ ] **Systemd detection edge cases**: Does `INVOCATION_ID` work correctly inside Crostini's systemd? (Crostini runs its own systemd instance, should be fine.)
- [ ] **Multiple roots UX**: How should the extension present multiple download roots? Dropdown when adding a torrent? Default root with option to change?

## Development workflow

The standalone mode can be developed and tested entirely on the dev machine (Mac):

1. Build and run `io-daemon --standalone` locally
2. Point browser at localhost dev server or website
3. Iterate on folder browser, roots management, self-update UI
4. Deploy to Crostini later — same binary, same protocol, just different default paths

This avoids the slow Chromebook deploy cycle for most development.

## Implementation order (suggested)

1. **Config persistence** — `~/.config/jstorrent-standalone/config.json`, load/save roots
2. **`GET /browse`** — directory listing endpoint
3. **`POST /roots`** — add/remove roots, persist
4. **Folder browser modal** — client-side UI
5. **Self-update endpoint** — `POST /self-update`, systemd detection
6. **Update prompt UI** — version comparison, changelog display
7. **Metrics check-in** — background tokio task
8. **Install script updates** — `--download-root` flag, config preservation
9. **Website client** — serve UI from jstorrent.com (separate effort, larger scope)
