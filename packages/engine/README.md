# `@jstorrent/engine`

`@jstorrent/engine` is JSTorrent's shared TypeScript BitTorrent engine. The
same protocol implementation runs in:

- the Chrome extension and Tauri webview with daemon-backed IO
- Android QuickJS and iOS JavaScriptCore with native bindings
- Node.js as a library, CLI, test runtime, and reference daemon

Platform adapters implement abstract filesystem, socket, hashing,
configuration, and session interfaces. The core torrent and peer logic does
not depend on a particular UI.

## CLI

```bash
npm install -g @jstorrent/engine
jstorrent "magnet:?xt=urn:btih:..." -o ~/Downloads
jstorrent ./file.torrent --download-path /data
jstorrent --help
```

The CLI can continue seeding with `--seed` and exposes connection, port, and
logging controls through `--help`.

## Library

```ts
import { createNodeEngine } from '@jstorrent/engine/presets/node'

const engine = createNodeEngine({
  downloadPath: './downloads',
  port: 6881,
})

const { torrent } = await engine.addTorrent('magnet:?xt=urn:btih:...')

while (torrent.progress < 1) {
  await new Promise((resolve) => setTimeout(resolve, 1000))
}

await engine.destroy()
```

Available package entry points include the main library, Node and native
presets, native adapters, daemon control connection, and Node IO daemon.

## Current Capabilities

- BitTorrent wire protocol, magnet metadata exchange, fast extension, and PEX
- HTTP, HTTPS, and UDP trackers
- IPv4 and IPv6 networking
- DHT and local peer discovery
- MSE/PE and SOCKS5 proxy support
- rarest-first selection, endgame, request pipelining, choking, and upload
- file priorities, session persistence, recheck, and torrent queueing
- streaming-aware scheduling and BEP 19 web seeds
- UPnP, NAT-PMP, and PCP port mapping
- daemon, memory, native, Node, null, and browser adapters

## Source Map

- `src/core/`: engine, torrent, peer, piece, queue, storage, and scheduling
- `src/adapters/`: browser, daemon, memory, native, Node, and null backends
- `src/dht/`, `src/tracker/`, `src/extensions/`: peer discovery and protocol extensions
- `src/http/`, `src/webseed/`, `src/proxy/`: HTTP data paths and proxying
- `src/node-io-daemon/`: Node reference daemon
- `src/node-rpc/`: Node controller and RPC surface
- `src/presets/`: supported runtime compositions
- `integration/python/`: libtorrent-backed and end-to-end integration tests

Superseded architecture plans and migration records are preserved under
`docs/archive/engine/`.

## Validation

From the repository root:

```bash
pnpm --filter @jstorrent/engine typecheck
pnpm --filter @jstorrent/engine test
pnpm --filter @jstorrent/engine test:python
pnpm conformance:daemon
pnpm conformance:native-host
```

The human-readable protocol contracts are:

- [`docs/contracts/io-daemon-contract.md`](../../docs/contracts/io-daemon-contract.md)
- [`docs/contracts/native-host-contract.md`](../../docs/contracts/native-host-contract.md)

## Info Hash Invariant

External hash strings must go through `infoHashFromHex`; binary hashes must go
through `infoHashFromBytes`. Do not cast arbitrary strings to `InfoHashHex`.

## Release

Engine releases use `./scripts/release-engine.sh <version>`. The script commits,
pushes, and tags the release; CI publishes to npm. Read the
[release topic](../../docs/topics/releases.md) before running it.
