# JSTorrent Chrome Extension

The Chrome MV3 extension runs the shared TypeScript BitTorrent engine in its
foreground UI page. Its service worker handles browser integration and
bootstrap; privileged file and socket operations come from either the desktop
native host or the paired Android companion on ChromeOS.

## Source Map

- `src/sw.ts`: MV3 service worker and external message routing
- `src/ui/`: main client and sharing pages
- `src/lib/native-connection.ts`: desktop native messaging bootstrap
- `src/lib/chromeos-bootstrap.ts`: Android companion discovery and pairing
- `src/magnet/`: browser magnet-link entry
- `public/manifest.json`: source manifest and release version
- `e2e/`: Playwright extension tests

The visible application is mounted from `@jstorrent/client`; high-frequency
tables come from `@jstorrent/ui`.

## Development

Install dependencies from the repository root, then run:

```bash
pnpm --filter jstorrent-extension dev
```

This starts the extension build watcher and the web UI development server. See
[`DEVELOPMENT.md`](../DEVELOPMENT.md) for the local hostname, daemon CORS, and
unpacked-extension setup.

Useful package commands:

```bash
pnpm --filter jstorrent-extension build
pnpm --filter jstorrent-extension typecheck
pnpm --filter jstorrent-extension test
pnpm --filter jstorrent-extension test:e2e
```

Build output is written to `extension/dist`.

## ChromeOS Hardware

Use the repository-level deployment entry point:

```bash
./scripts/deploy-chromebook.sh
```

It builds, deploys directly to ChromeOS Downloads through `chromeroot`, and
reloads the unpacked extension through DevTools. It does not require Crostini.
See the
[ChromeOS hardware-testing topic](../docs/topics/chromeos-hardware-testing.md)
for testbed ownership, first-load setup, UI assertions, and recovery.

## Packaging

Create a Chrome Web Store ZIP without the development manifest key:

```bash
./scripts/package-extension.sh
```

The output is `extension/package.zip`. Store-listing promotional images are
not included in the extension package.

## Current Documentation

- [Search plugin authoring and runtime](../docs/topics/search-plugins.md)
- [Sandbox and trust boundaries](../docs/topics/sandbox-and-search-plugin-trust-boundaries.md)
- [Native host contract](../docs/contracts/native-host-contract.md)
- [IO daemon contract](../docs/contracts/io-daemon-contract.md)

The old extension design and implementation walkthrough are retained under
`docs/archive/extension/`.

## Release

Extension releases use:

```bash
./scripts/release-extension.sh <version>
```

The script commits, pushes, and tags the release. CI creates the ZIP and GitHub
release; Chrome Web Store upload remains manual. Read the
[release topic](../docs/topics/releases.md) before running it.
