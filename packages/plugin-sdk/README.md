# `@jstorrent/plugin-sdk`

The plugin SDK contains the public types, validators, and Node test runtime for
JSTorrent search plugins.

Install it for plugin development:

```bash
npm install @jstorrent/plugin-sdk
```

The default entry is browser-safe and exports manifest, fetch-policy, result,
and source validation. Node-only loading, HTML parsing, transformation, and
execution are exported from `@jstorrent/plugin-sdk/node`. Test contexts and
mock fetch routes are available from `@jstorrent/plugin-sdk/testing`.

The installed `jstorrent-plugin` command validates manifests and source,
executes plugin tests, and inspects plugin output. Run
`jstorrent-plugin --help` for the current commands and options.

Package development:

```bash
pnpm --filter @jstorrent/plugin-sdk build
pnpm --filter @jstorrent/plugin-sdk typecheck
pnpm --filter @jstorrent/plugin-sdk test
```

The current plugin manifest, runtime API, result shape, and reference
implementation are documented in
[`docs/topics/search-plugins.md`](../../docs/topics/search-plugins.md).

SDK releases use `./scripts/release-plugin-sdk.sh <version>`. Read the
[release topic](../../docs/topics/releases.md) before running it.
