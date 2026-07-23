# JSTorrent Desktop

`desktop/` contains the Rust native subsystem and the Tauri desktop product for
macOS, Windows, and Linux.

The same binaries support two user configurations:

- The Chrome extension starts `jstorrent-host` through native messaging. The
  host owns profiles and download roots and launches `jstorrent-io-daemon`.
- The Tauri app bundles the shared React client with both sidecars, registers
  the native messaging host, handles links and files, and can run standalone.

## Workspace

| Component | Role |
| --- | --- |
| `common` | Shared profile, path, and protocol data structures |
| `host` | Native messaging bootstrap, profile ownership, roots, KV storage, and daemon lifecycle |
| `io-daemon` | Authenticated HTTP/WebSocket file, socket, hash, media, and control services |
| `tauri-app/src-tauri` | Desktop window, sidecar management, updater, tray, deep links, and browser registration |
| `tauri-app` | Vite/React frontend that mounts `@jstorrent/client` |

There is no separate tracked `jstorrent-link-handler` crate. Magnet and torrent
file handling now live in the Tauri application and native host.

## Prerequisites

- Rust stable
- Node.js and pnpm
- platform dependencies required by Tauri

Install JavaScript dependencies from the repository root:

```bash
pnpm install
```

## Rust Build and Tests

From `desktop/`:

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Release sidecar binaries are written to `desktop/target/release/`.

The shared protocol gates can be run from the repository root:

```bash
pnpm conformance:daemon
pnpm conformance:native-host
```

## Tauri Development

From the repository root:

```bash
pnpm --dir desktop/tauri-app tauri dev
```

The package's `tauri` script first builds and copies the host and IO-daemon
sidecars with the names Tauri expects. Details of the resolution chain and
stale-binary recovery are in
[`tauri-app/SIDECARS.md`](tauri-app/SIDECARS.md).

Local installation helpers:

```bash
desktop/scripts/install-local-linux-sidecars.sh
desktop/scripts/install-local-tauri-linux.sh
desktop/scripts/install-local-tauri-macos.sh
desktop/scripts/install-local-tauri-pkg-macos.sh
```

Use the helper that matches whether the extension sidecars, standalone Tauri
app, or macOS package is being tested.

## Configuration and Logs

| Platform | Configuration directory |
| --- | --- |
| macOS | `~/Library/Application Support/jstorrent-native/` |
| Linux | `~/.config/jstorrent-native/` |
| Windows | `%LOCALAPPDATA%\\jstorrent-native\\` |

Important files include profile-scoped runtime information, the optional
`jstorrent-native.env` developer override file, and
`jstorrent-native-host.log` / `io-daemon.log`.

[`jstorrent-native.env.example`](jstorrent-native.env.example) documents the
supported local overrides.

## Architecture and Contracts

- [`docs/contracts/native-host-contract.md`](../docs/contracts/native-host-contract.md):
  native messaging and profile behavior
- [`docs/contracts/io-daemon-contract.md`](../docs/contracts/io-daemon-contract.md):
  daemon HTTP/WebSocket behavior
- [`docs/topics/sandbox-and-search-plugin-trust-boundaries.md`](../docs/topics/sandbox-and-search-plugin-trust-boundaries.md):
  desktop capability and search-plugin boundaries

Superseded desktop designs and installer plans are retained under
`docs/archive/desktop/`.

## Release

Desktop releases use:

```bash
./scripts/release-tauri-app.sh <version>
```

The script commits, pushes, and tags the release. Read the
[release topic](../docs/topics/releases.md) before running it.
