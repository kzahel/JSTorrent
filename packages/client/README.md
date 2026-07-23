# `@jstorrent/client`

`@jstorrent/client` is the React application and platform bridge shared by the
Chrome extension, Tauri desktop app, and browser-hosted client page. Android
and iOS use native UIs and do not run this package as their application shell.

## Responsibilities

- mount the shared React application
- create and expose the in-page engine adapter
- bootstrap desktop native messaging or ChromeOS companion IO
- provide host-channel abstractions for Chrome and Tauri
- manage settings, search plugins, notifications, and video playback
- bridge React layout to the high-frequency Solid tables in `@jstorrent/ui`

## Entry Points

| Export | Intended use |
| --- | --- |
| `@jstorrent/client` | Full Chrome/Tauri application and browser integrations |
| `@jstorrent/client/core` | Chrome-free contexts, hooks, types, and application content |
| `@jstorrent/client/video-popup` | Standalone video popup component |

## Source Map

- `src/App.tsx`: platform-aware application wrapper
- `src/AppContent.tsx`: Chrome-free shared application content
- `src/engine-manager/`: daemon-backed engine lifecycle
- `src/host/`: Chrome and Tauri host channels
- `src/context/`: engine, configuration, and search-plugin React contexts
- `src/hooks/`: engine state, bootstrap, configuration, and bridge hooks
- `src/search/`: plugin service, sandbox host, validation utilities, and types
- `src/components/`: shell, settings, search, system bridge, and playback UI

## Development

From the repository root:

```bash
pnpm --filter @jstorrent/client build
pnpm --filter @jstorrent/client typecheck
pnpm --filter @jstorrent/client test
pnpm --filter @jstorrent/client dev
```

`dev` runs the TypeScript compiler in watch mode. Use the extension or Tauri
development command to run the actual application.

## Environment Boundaries

- The Chrome extension imports the full package and supplies Chrome native
  messaging and notification capabilities.
- The Tauri app imports the same full application and registers Tauri URL
  opening before mounting it.
- Browser-hosted surfaces should prefer `@jstorrent/client/core` when Chrome
  APIs are unavailable.
- Native Android and iOS frontends communicate directly with
  `@jstorrent/engine` through their embedded JavaScript runtimes.

Search-plugin contracts and examples live in
[`docs/topics/search-plugins.md`](../../docs/topics/search-plugins.md).
