# ChromeOS Crostini No-Play-Store Exploratory Test

Date: 2026-07-24

Result: Pass; the testbed startup issue found during the run was fixed

## Outcome

The published no-Play-Store route completed on a physical Chromebook:

`fresh Crostini daemon install -> automatic discovery -> automatic pairing ->
Ready`

The exact public command installed and started the standalone daemon:

```bash
curl -fsSL https://jstorrent.com/install-crostini.sh | bash
```

With the Android companion force-stopped, the extension's cached companion
host, port, and token removed, and the daemon's pairing state reset, opening
the extension automatically paired it to `penguin.linux.test:7800`. The System
Bridge panel reported **Crostini Daemon**, **Connected**, and the `Downloads`
download location.

This pass validated installation, health, discovery, pairing, root discovery,
the control WebSocket, and capability negotiation. It did not run a torrent
payload transfer.

## Environment

| Component | Value |
| --- | --- |
| Repository revision | `b5e9939da80c21c5e5e55d963472a8b07900f001` |
| ChromeOS | Official build 16700.46.0, milestone 150 |
| Board | `nami-signed-mp-v12keys` |
| Extension | unpacked MV3 1.1.1, `dbokmlpefliilbjldladbimlcfgbolhk` |
| Standalone daemon | 0.2.1, x86_64 |
| Daemon endpoint | `penguin.linux.test:7800` |
| Download root | Crostini `~/Downloads` |
| Android companion | installed but force-stopped during the acceptance test |

ChromeOS Flex hardware was not available for this run. The exercised backend
and user flow are the same Crostini route advertised for devices without ARC.

## Published-Artifact Checks

- The live FAQ displayed the same one-line command as the checkout.
- The live `install-crostini.sh` exactly matched
  `website/public/install-crostini.sh`, with SHA-256
  `c838e4f9f6b48423b3bf8bfd5a6f5455dbb2fb56eb2bd1220f82a293579d7861`.
- The installer selected `tauri-app-v0.2.1`.
- Both published standalone assets returned HTTP 200:
  `jstorrent-io-daemon-x86_64-unknown-linux-gnu` and
  `jstorrent-io-daemon-aarch64-unknown-linux-gnu`.
- Release 0.2.1 predates release-hosted `SHA256SUMS`, so the expected release
  manifest request returned 404. The installer then used the documented
  website bootstrap manifest and verified both binaries successfully.
- `bash -n website/public/install-crostini.sh` passed.
- `./scripts/test-release-integrity.sh` passed all success, mismatch, missing
  manifest, fallback manifest, and missing-entry cases for both public
  installers.

## Physical Flow

1. Crostini initially had no `~/.local/bin/jstorrent-io-daemon`, no
   `jstorrent-io.service`, no standalone pairing config, and nothing listening
   on port 7800.
2. The published command downloaded the x86_64 0.2.1 daemon, verified its
   checksum, installed the systemd user service, and passed its localhost
   health check.
3. Android was force-stopped so it could not win extension discovery.
4. Crostini was restarted through the normal ChromeOS Terminal `penguin`
   connection. Both `http://localhost:7800/health` and
   `http://penguin.linux.test:7800/health` then returned HTTP 200 from ChromeOS.
5. The extension daemon host, port, auth token, and prior-success markers were
   cleared. The standalone daemon's pairing config was removed and its service
   restarted; `/status` confirmed `paired: false`.
6. A fresh extension UI target was opened. Without a host override or manual
   retry, the daemon auto-approved the extension, `/status` changed to
   `paired: true`, and the extension reached **Ready**.
7. The System Bridge panel identified the backend as Crostini rather than
   Android and exposed the fixed `Downloads` root.

## Testbed Startup Finding and Resolution

The first attempt produced a useful false negative. At the time, the testbed's
`chromeos crostini-start` command started `penguin` directly with `lxc` so that
it could restore its SSH forwarding. On a cold VM this bypassed the normal
ChromeOS guest-registration handshake:

- `cros-garcon.service` repeatedly reported that it could not read its security
  token.
- `penguin.linux.test` did not resolve.
- ChromeOS localhost tunneling was absent.
- The daemon remained healthy at the container's numeric IP, but the extension
  could not discover it using the production hostname.

Stopping Crostini and starting `penguin` through the actual Terminal app
restored garcon registration, hostname resolution, and localhost tunneling.
Therefore a product acceptance test for this route must use the user-facing
Terminal startup path. A direct numeric-IP override is not an adequate
substitute: it can connect and pair, but current UI backend classification
recognizes Crostini by the `penguin.linux.test` hostname and otherwise shows an
incorrect Android **Update Required** state for daemon 0.2.1.

The testbed was corrected after this run. `crostini-start` now opens Launcher,
starts the ChromeOS Terminal app, selects its `penguin` profile, waits for
`penguin.linux.test` to resolve, restores SSH forwarding, and verifies
authenticated SSH. `crostini-stop` likewise uses Terminal's shelf-menu
**Shut down Linux** action. A physical cold start, verified shutdown, and
second cold start all passed with those user-facing paths.

## Artifacts

Artifacts were collected locally under:

```text
/tmp/jstorrent-crostini-route-20260724
```

High-signal frames are:

- `extension-after-crostini-install.png` — false Offline result after the
  direct-`lxc` testbed start
- `terminal-proper-start.png` — normal Terminal-backed Crostini session
- `extension-fresh-natural-crostini.png` — fresh automatic pairing at Ready
- `extension-fresh-crostini-panel.png` — Crostini daemon endpoint and download
  root

## Cleanup

The published uninstall command removed the test daemon, service, and pairing
config. Crostini was then shut down and its testbed forwarding removed. The
Android companion was relaunched and the extension returned to **Ready** at
`100.115.92.2:7800` with its existing `JSTorrent` SAF root. The pre-existing
PhysBox tab was also restored.

## Remaining Coverage

- Run the same scenario on actual ChromeOS Flex hardware.
- Add a deterministic torrent transfer and independent hash comparison.
- Preserve the testbed contract that Crostini startup goes through Terminal
  and verifies `penguin.linux.test` before a product assertion.
- Keep the published post-reboot advice prominent: opening a Terminal
  `penguin` session is what starts Crostini and restores the ChromeOS tunnels.
