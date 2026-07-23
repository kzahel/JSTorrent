# `@jstorrent/ui`

`@jstorrent/ui` provides JSTorrent's shared presentation components, virtualized
tables, settings hooks, and formatting utilities. The extension, Tauri app, and
browser-hosted client consume its TypeScript source directly.

React owns the application layout and component lifecycle. High-frequency
tables mount Solid components through `TableMount`, read current engine data
through callbacks, and refresh with a throttled animation-frame loop. This
keeps rapidly changing torrent and peer data out of React state.

## Package Surface

- `src/components/`: detail panes, dialogs, menus, file selection, toasts,
  speed graphs, and piece visualization
- `src/tables/`: torrent, peer, swarm, file, piece, tracker, disk, and log
  tables plus the React-to-Solid mount
- `src/hooks/`: persisted layout, selection, theme, scale, and frame-rate
  settings
- `src/storage/`: browser storage abstraction for UI settings
- `src/utils/`: formatting, country flags, and animation-frame throttling
- `src/styles.css`: shared application theme and layout styles

The detail pane currently supports general, tracker, peer, swarm, file, piece,
disk, search, log, speed, and DHT views. Not every internal table is exported
as a standalone public component; use [`src/index.ts`](src/index.ts) as the
package API.

## Table Model

`TableMount` bridges a React parent to the Solid `VirtualTable`. Callers supply
row and selection getters so the table always reads the latest values:

```tsx
<TableMount
  getRows={() => source.rows}
  getRowKey={(row) => row.id}
  columns={columns}
  storageKey="example"
  getSelectedKeys={() => selectedKeys}
  onSelectionChange={setSelectedKeys}
/>
```

Column visibility, width, ordering, and sort settings persist per storage key.
The refresh loop respects the application's maximum-FPS setting and reduces
work when a table has no active rows.

## Development

From the repository root:

```bash
pnpm --filter @jstorrent/ui test
pnpm --filter @jstorrent/ui test:watch
pnpm lint
```

The package does not have a standalone build. Vite compiles React and Solid JSX
for the consuming application; the workspace `typecheck` command records the
package's intentional JSX-transform skip.
