# JSTorrent Development

This is the maintainer entry point for the JSTorrent monorepo. Product
architecture and user-facing capabilities are summarized in
[`README.md`](README.md); continuing concerns and historical context are
indexed from [`docs/README.md`](docs/README.md).

## Prerequisites

- Git with submodule support
- Node.js 20 or newer
- pnpm 9 through Corepack
- Platform toolchains for the component being changed:
  - Rust stable for `desktop/`
  - JDK 17 and the Android SDK for `android/`
  - Xcode and XcodeGen for `ios/`
  - Python 3.10+ and `uv` for Python integration tests

On the primary development machines, `source ~/.profile` loads the configured
Java, Android, Rust, and other toolchain paths.

## Repository Map

| Path | Role |
| --- | --- |
| `extension/` | Chrome MV3 service worker, pages, build, and browser integration |
| `packages/engine/` | Shared TypeScript BitTorrent engine and Node CLI |
| `packages/client/` | React client shared by the extension and Tauri app |
| `packages/ui/` | React/Solid presentation components |
| `packages/plugin-sdk/` | Search-plugin types, validation, and Node test runtime |
| `desktop/` | Rust native host, IO daemon, shared crate, and Tauri desktop app |
| `android/` | Native Compose app, QuickJS runtime, and ChromeOS companion daemon |
| `ios/` | Native SwiftUI app, JavaScriptCore runtime, and release tooling |
| `website/` | Astro website and browser-hosted client entry |
| `contracts/` | Machine-readable native-host and IO-daemon contracts |
| `docs/` | Living topics, normative contracts, reference material, and archive |
| `scripts/` | Repository-level validation, release, deployment, and utility scripts |
| `update-server/` | Product configuration for the external desktop update service |

## Initial Setup

```bash
git submodule update --init --recursive
corepack enable
pnpm install
```

The Android QuickJS module uses the `quickjs-ng` submodule, so builds from a
fresh clone need the submodule initialization step.

## Root Commands

| Command | Purpose |
| --- | --- |
| `pnpm lint` | Run ESLint |
| `pnpm format` | Check formatting with Prettier |
| `pnpm format:fix` | Rewrite files with Prettier |
| `pnpm docs:check` | Validate active documentation links, paths, and portability |
| `pnpm typecheck` | Run workspace TypeScript checks |
| `pnpm test` | Run workspace unit tests |
| `pnpm test:python` | Run engine Python integration tests |
| `pnpm build` | Build workspace packages |
| `pnpm checkall` | Run static checks and test suites |

Use the smallest relevant package command while iterating, then run the
repository checks appropriate to the final change.

## Development Servers

`pnpm dev` recursively starts every workspace with a `dev` script:

- the Astro website at <http://localhost:3000>
- the extension web UI at <http://local.jstorrent.com:3001>
- the extension build watcher
- the client TypeScript watcher
- the Tauri frontend Vite server at <http://localhost:1420>

For focused work, prefer a package command:

```bash
pnpm --filter jstorrent-extension dev
pnpm --filter jstorrent-website dev
pnpm --filter @jstorrent/client dev
```

The Tauri native application uses:

```bash
pnpm --dir desktop/tauri-app tauri dev
```

For extension web development, map `local.jstorrent.com` to loopback:

```text
127.0.0.1 local.jstorrent.com
```

If the native daemon rejects the development origin, add this to the platform's
`jstorrent-native.env` and restart the native host:

```text
DEV_ORIGINS=http://local.jstorrent.com:3001
```

Build the extension once and load `extension/dist` from
`chrome://extensions` as an unpacked extension.

## Platform Entry Points

- [Chrome extension development](extension/README.md)
- [Desktop and native-host development](desktop/README.md)
- [Android and ChromeOS development](android/README.md)
- [iOS development](ios/README.md)
- [Engine package and CLI](packages/engine/README.md)
- [Shared client package](packages/client/README.md)
- [Search plugin SDK](packages/plugin-sdk/README.md)
- [Website development](website/README.md)
- [Release operations](docs/topics/releases.md)

## CI

GitHub Actions workflows under `.github/workflows/` are path-filtered by
component. The main families cover:

- repository static checks
- extension build and Playwright tests
- Android builds, unit tests, instrumentation tests, and conformance
- Rust/native-host conformance
- Tauri desktop installers and updater artifacts
- iOS build, tests, notarization, and AltStore publication
- engine and plugin SDK npm publication
- website deployment to GitHub Pages

The release topic maps each tag to its workflow and remaining manual steps.
