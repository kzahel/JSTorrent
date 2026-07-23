# ChromeOS Flex Support

**Status:** Implemented (CI + install script)
**Priority:** Low effort, nice-to-have
**Audience:** ChromeOS users without ARC / Play Store (ChromeOS Flex, rare devices without ARC)

---

## The Gap

ChromeOS Flex runs on regular PCs and has Chrome + Crostini (Linux container), but **no ARC** — so the Android companion app isn't available. Today these users have no supported path to use JSTorrent.

The Chrome extension provides the UI and engine, but it needs an I/O backend for sockets and file access. On regular ChromeOS that's the Android companion. On desktop that's the Rust native host. On ChromeOS Flex, neither is available out of the box.

## Solution: io-daemon in Crostini

Crostini gives us a full Linux environment. The io-daemon binary already supports standalone mode (direct WebSocket, no native messaging). We just need to make installation trivial.

### One-liner install

```bash
curl -fsSL https://jstorrent.com/install-crostini.sh | bash
```

**Script:** `website/public/install-crostini.sh`

The script:
1. Detects architecture (x86_64 or aarch64)
2. Fetches the latest release tag from GitHub API
3. Downloads the `io-daemon` Linux binary from GitHub Releases
4. Places it in `~/.local/bin/`
5. Creates a systemd user service (`~/.config/systemd/user/jstorrent-io.service`)
6. Enables lingering (`loginctl enable-linger $USER`) so the service survives terminal close
7. Starts the service and verifies health

Supports `--uninstall` and `--version X` flags. Idempotent — running again updates the binary.

**CI:** The `tauri-app-ci.yml` workflow uploads standalone `jstorrent-io-daemon-{triple}` binaries to each Tauri App GitHub Release (Linux x86_64 and ARM64).

### systemd service

```ini
[Unit]
Description=JSTorrent I/O Daemon

[Service]
ExecStart=%h/.local/bin/jstorrent-io-daemon --standalone
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

With `enable-linger`, the user service manager starts at boot (when Crostini is running) without needing an active terminal session.

### Extension integration

The extension already probes `penguin.linux.test` (Crostini hostname) as a fallback after the ARC host (`100.115.92.2`) fails. Once the daemon is running in Crostini, the extension auto-pairs and connects with no additional user interaction.

**TODO:** Add UI in `SystemBridgePanelChromeos` to surface the Crostini install option when ARC probing fails:
- "No Play Store?" expandable section with the one-liner install command
- Connection status indicator (daemon reachable or not)
- "Start Crostini" hint if the daemon isn't reachable

## Known Limitation

Crostini itself must be running. After a full reboot, the container doesn't auto-start — the user has to open the Terminal app or any Linux app once. This is an OS-level constraint. The extension can detect this and prompt accordingly.

## Effort

Minimal — the io-daemon binary and standalone WebSocket mode already exist. The install script and CI binary upload are done. Remaining work is extension UI to surface the Crostini option.
