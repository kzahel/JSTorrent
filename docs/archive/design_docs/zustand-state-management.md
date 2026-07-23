# Zustand State Management

## Problem

Several components manage complex state with many `useState` calls, manual `localStorage` persistence, and prop-drilling. Search results are lost when switching tabs because the SearchTab component unmounts and its React state is discarded.

## Why Zustand

The project currently has no external state management library. State lives in component-level `useState` + `useEffect`, with manual `localStorage` read/write for persistence. This works but creates friction:

- **State lost on unmount** — SearchTab results disappear on tab switch
- **Manual persistence boilerplate** — each component hand-rolls `JSON.stringify`/`parse`, error handling, key management
- **Prop-drilling** — App.tsx passes modal flags, root selection, and callbacks through multiple levels

Zustand is a good fit because:

- ~1KB, no Provider wrapper, no boilerplate
- `persist` middleware replaces all manual localStorage code
- Stores are module-level singletons — state survives component unmount
- Works outside React (in event handlers, utilities)
- Already the standard choice for new React projects

## Current State Patterns & Migration Plan

### Priority 1: Search Stores (immediate)

**SearchTab.tsx** — 10+ `useState` calls: `query`, `results`, `summaries`, `searching`, `addingKey`, `status`, `selectedRows`, `plugins`, `selectedPluginIds`, `contextMenu`. Manual localStorage for `selectedPluginIds`. Results lost on tab switch.

→ `useSearchStore` with `persist` middleware for query/results/selectedPluginIds. Solves the tab-switching problem directly.

**SearchPluginsOverlay.tsx** — Many `useState` calls across 4 sub-tabs (search/installed/add/lab). Manual localStorage for `searchInput` and `selectedPluginIds` under key `jstorrent:searchPluginsOverlayState`.

→ `useSearchPluginsStore` with `persist` for searchInput/selectedPluginIds. Volatile state (labBusy, draftRunResult) stays unpersisted in the store.

### Priority 2: App-Level Store (next)

**App.tsx** — Modal visibility (`settingsOpen`, `searchPluginsOpen`), `settingsTab`, `defaultRootKey`, engine init state. These are prop-drilled to child components.

→ `useAppStore` consolidating modal states and root selection. Eliminates prop chains.

### Priority 3: UI Persistence (optional)

**usePersistedUIState** — Detail pane height + active tab with manual localStorage + resize handling.

→ Zustand `persist` middleware replaces the custom storage code.

### Not Migrating

The 5 Context providers (Engine, EngineManager, Config, SearchPluginService, HostChannel) are dependency injection patterns — React context is the right tool for those. No change needed.

## Store Conventions

- Store files live in `packages/client/src/stores/`
- One file per store, named `use<Name>Store.ts`
- localStorage keys prefixed with `jstorrent:` (matching existing convention)
- Only persist state that should survive page reload; transient UI state (loading flags, context menus) stays unpersisted
- Selectors in components: `const results = useSearchStore((s) => s.results)` — pick individual fields to minimize re-renders
