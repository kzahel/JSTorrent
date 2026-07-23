# Tauri Desktop App: HostChannel Design

## Goal

Introduce a **HostChannel** abstraction in `packages/client/` that replaces the Chrome-specific `ExtensionBridge` with a host-agnostic interface for all UI-to-host communication. Wire up the Tauri desktop app to use the existing system-bridge (native host) binary via this interface, reusing the same architecture as the Chrome extension.

## Motivation

The JSTorrent UI runs in multiple contexts, all needing the same host services (daemon connection, KV storage, file operations, notifications). Today these are all routed through `chrome.runtime.sendMessage` / `chrome.runtime.Port`, which only works when the Chrome extension is present.

The Tauri desktop app renders the shared `@jstorrent/client` UI but has no Chrome extension. It needs a different transport to the same backend services. Rather than duplicate logic, we introduce a clean interface and make each context plug in its own transport.

## Deployment Contexts

| # | Context | UI Transport | Backend |
|---|---------|-------------|---------|
| 1 | **Desktop extension** | Chrome SW (internal msg) | SW → native messaging → system-bridge → io-daemon |
| 2 | **ChromeOS + ARC** | Chrome SW (internal msg) | SW → WebSocket → Android companion |
| 3 | **ChromeOS Flex / Crostini** | Chrome SW (internal msg) | SW → WebSocket → standalone io-daemon |
| 4 | **Tauri desktop** | Tauri invoke | Rust backend → stdin/stdout → system-bridge → io-daemon |
| 5 | **Website** | Chrome SW (external msg) | Same as 1/2/3 via extension |

Contexts 1–3 and 5 route through the Chrome extension service worker. The SW exists because Chrome requires it for native messaging — it's a platform constraint, not an architectural choice.

Context 4 (Tauri) replaces the SW with the Tauri Rust backend speaking the same native messaging protocol to the same system-bridge binary.

## Architecture

### Current (Chrome Extension)

```
UI (React)
  → chrome.runtime.sendMessage / Port
    → Service Worker
      → chrome.runtime.connectNative (4-byte LE length + JSON)
        → system-bridge (native host)
          → spawns io-daemon
          → handles: file picker, roots, file ops, link events
```

### Proposed (Tauri)

```
UI (React)
  → invoke('host_message', { message })
    → Tauri Rust backend (lib.rs)
      → stdin/stdout (4-byte LE length + JSON) — same protocol
        → system-bridge (native host) — same binary, unmodified
          → spawns io-daemon
          → handles: file picker, roots, file ops, link events
```

### Process Hierarchy

```
Tauri App (GUI, webview, system tray)
  └─ system-bridge (sidecar, stdin/stdout IPC)
       └─ io-daemon (spawned by system-bridge, TCP/UDP/HTTP)
```

The Tauri app replaces both Chrome and the link-handler binary:
- **Chrome's role** (launching native host, relaying messages) → Tauri Rust backend
- **Link-handler's role** (magnet: / .torrent protocol handler) → Tauri deep-link plugin

## HostChannel Interface

```typescript
type Unsubscribe = () => void

interface HostChannel {
  // --- Lifecycle ---
  connect(): Promise<void>
  disconnect(): void

  // --- Connection state (reactive) ---
  getState(): HostState
  onStateChanged(cb: (state: HostState) => void): Unsubscribe

  // --- Events from host (TorrentAdded, MagnetAdded) ---
  onEvent(cb: (event: NativeEvent) => void): Unsubscribe

  // --- Capabilities ---
  readonly capabilities: HostCapabilities

  // --- KV storage ---
  kvGet<T = unknown>(key: string, opts?: KVOpts): Promise<T | undefined>
  kvGetMulti(keys: string[], opts?: KVOpts): Promise<Record<string, unknown>>
  kvSet(key: string, value: unknown, opts?: KVOpts): Promise<void>
  kvDelete(key: string, opts?: KVOpts): Promise<void>
  kvKeys(prefix?: string, opts?: KVOpts): Promise<string[]>
  kvClear(prefix?: string, opts?: KVOpts): Promise<void>

  // --- File operations ---
  pickDownloadFolder(): Promise<DownloadRoot | null>
  removeDownloadRoot(key: string): Promise<void>
  openFile(rootKey: string, path: string): Promise<void>
  revealInFolder(rootKey: string, path: string): Promise<void>

  // --- Notifications (UI → host, for native OS notifications) ---
  notify(notification: HostNotification): void

  // --- Host actions ---
  retryConnection(): void
  triggerLaunch(): void

  // --- Debug / admin ---
  getStats(): Promise<DaemonStats | null>
  getDaemonInfo(): Promise<DaemonInfo | null>
  clearSessionStorage(): Promise<void>
  notifyClosing(): void

  // --- App info ---
  getVersion(): string | null
  isDevMode(): boolean
  requestPermission(permission: string): Promise<boolean>
}
```

### Supporting Types

```typescript
interface HostState {
  status: 'connecting' | 'connected' | 'disconnected'
  platform: 'desktop' | 'chromeos' | 'tauri'
  daemonInfo: DaemonInfo | null
  roots: DownloadRoot[]
  lastError: string | null
}

interface HostCapabilities {
  rootsManageable: boolean          // Can add/remove download roots
  hasSync: boolean                  // KV sync storage available
  hasNativeNotifications: boolean   // Can show OS-level notifications
  hasBackgroundPersistence: boolean // Stays alive without UI tricks
}

interface KVOpts {
  keyPrefix?: string                // Key namespace (default: 'session:')
  area?: 'local' | 'sync'          // Chrome storage area; Tauri ignores
}

type HostNotification =
  | { type: 'visibility'; visible: boolean }
  | { type: 'stats'; stats: ProgressStats }
  | { type: 'torrent-complete'; infoHash: string; name: string }
  | { type: 'torrent-error'; infoHash: string; name: string; error: string }
  | { type: 'duplicate-torrent'; name: string }

interface NativeEvent {
  event: string
  payload: unknown
}
```

## Implementations

### ChromeExtensionChannel (contexts 1–3, 5)

Wraps `chrome.runtime.sendMessage` / `chrome.runtime.connect` with typed methods. The Service Worker continues to handle all routing internally — no SW changes needed.

| HostChannel method | Chrome message |
|---|---|
| `connect()` | `chrome.runtime.connect({name:'ui'})` + `GET_BRIDGE_STATE` |
| `onStateChanged` | Port listener for `BRIDGE_STATE_CHANGED` |
| `onEvent` | Port listener for native events |
| `kvGet(key)` | `sendMessage({type:'KV_GET', key, keyPrefix})` |
| `kvSet(key, value)` | `sendMessage({type:'KV_SET', key, value, keyPrefix})` |
| `pickDownloadFolder()` | `sendMessage({type:'PICK_DOWNLOAD_FOLDER'})` |
| `openFile(rootKey, path)` | `sendMessage({type:'OPEN_FILE', rootKey, path})` |
| `notify(n)` | `postMessage({type:'notification:' + n.type, ...})` |
| `getVersion()` | `chrome.runtime.getManifest().version` |
| `requestPermission('power')` | `chrome.permissions.request({permissions:['power']})` |

Port reconnection logic (currently in `useIOBridgeState`) moves into this class.

**ChromeOS-specific methods** (not on HostChannel interface):
- `openChromeOSIntent()`, `resetChromeOSPairing()`, `onChromeOSBootstrapStateChanged()`
- Used by `useChromeOSBootstrap` via type narrowing

### TauriChannel (context 4)

Communicates with the system-bridge via the Tauri Rust backend, which relays messages over stdin/stdout using the native messaging protocol.

| HostChannel method | Implementation |
|---|---|
| `connect()` | `invoke('host_handshake')` → sends Handshake to system-bridge → returns DaemonInfo |
| `onStateChanged` | `listen('host-state-changed', cb)` |
| `onEvent` | `listen('host-event', cb)` — receives MagnetAdded, TorrentAdded from system-bridge |
| `kvGet/Set/etc` | `localStorage` with `jst:` prefix |
| `pickDownloadFolder()` | `invoke('host_message', {op:'pickDownloadDirectory'})` → system-bridge native dialog |
| `openFile(rootKey, path)` | `invoke('host_message', {op:'openFile', rootKey, path})` |
| `revealInFolder(rootKey, path)` | `invoke('host_message', {op:'revealInFolder', rootKey, path})` |
| `notify(n)` | No-op initially; later Tauri notification plugin |
| `getStats()` | `fetch('http://127.0.0.1:{port}/stats')` with auth header |
| `getDaemonInfo()` | Cached from handshake response |
| `getVersion()` | Build-time constant or `invoke('get_version')` |
| `isDevMode()` | `import.meta.env.DEV` |
| `requestPermission()` | Returns `true` (desktop apps have full permissions) |
| `clearSessionStorage()` | Clear `localStorage` keys with `jst:session:` prefix |

**Capabilities**: `{ rootsManageable: true, hasSync: false, hasNativeNotifications: false, hasBackgroundPersistence: true }`

## Tauri Rust Backend

### Current State

The Tauri Rust backend (`desktop/tauri-app/src-tauri/src/lib.rs`) currently:
- Spawns io-daemon directly as a sidecar
- Reads port from io-daemon's stdout
- Exposes `get_daemon_info()` invoke command
- Manages system tray

### New State

Replace direct io-daemon spawning with system-bridge spawning. The Rust backend becomes a native messaging relay:

```rust
// Pseudocode for new lib.rs structure

struct HostBridge {
    child_stdin: ChildStdin,   // Write requests
    daemon_info: OnceCell<DaemonInfo>,
    pending: DashMap<String, oneshot::Sender<Response>>,
}

// Startup:
// 1. Spawn system-bridge sidecar
// 2. Read stdout in background task, dispatch responses/events
// 3. Send Handshake, receive DaemonInfo
// 4. Ready for invoke calls

#[tauri::command]
async fn host_handshake(state: ...) -> Result<DaemonInfo, String> {
    // Send: { id, op: "handshake", extensionId: "tauri", installId: <uuid> }
    // Recv: { id, ok, type: "DaemonInfo", payload: { port, token, version, roots } }
}

#[tauri::command]
async fn host_message(state: ..., message: Value) -> Result<Value, String> {
    // Add unique id, write to stdin, await matching response on stdout
}
```

The stdout reader runs as a background tokio task:
- **Responses** (have `id` field matching a pending request): resolve the pending oneshot
- **Events** (have `event` field: `MagnetAdded`, `TorrentAdded`, `Log`): emit as Tauri events to the frontend

### Sidecar Configuration

In `tauri.conf.json`, replace io-daemon with system-bridge:

```json
{
  "bundle": {
    "externalBin": [
      "binaries/jstorrent-host",
      "binaries/jstorrent-io-daemon"
    ]
  }
}
```

Both binaries ship as sidecars. The Tauri app spawns system-bridge, which finds io-daemon in the same directory via `find_io_daemon_path()`.

### Deep Links (replaces link-handler binary)

Register the Tauri app as protocol handler for `magnet:` URIs and `.torrent` files using `tauri-plugin-deep-link`. When a deep link arrives:

1. Tauri receives the URL via the deep-link plugin
2. Forward to system-bridge's RPC server: `POST http://127.0.0.1:{rpc_port}/add-magnet?token={rpc_token}`
3. System-bridge pushes `MagnetAdded` event on stdout
4. Rust backend emits Tauri event → TauriChannel → UI

Alternatively, bypass the RPC server and write a new message type directly to stdin. But using the existing RPC server requires zero system-bridge changes.

## Native Messaging Protocol

The protocol between Tauri Rust backend and system-bridge is identical to Chrome native messaging:

### Frame Format
```
[0..3]  Message length (4 bytes, little-endian u32)
[4..N]  JSON payload (UTF-8)
```

### Request Format
```json
{
  "id": "uuid-string",
  "op": "pickDownloadDirectory"
}
```

```json
{
  "id": "uuid-string",
  "op": "handshake",
  "extensionId": "tauri-desktop",
  "installId": "persisted-uuid"
}
```

```json
{
  "id": "uuid-string",
  "op": "openFile",
  "rootKey": "abc123",
  "path": "downloads/movie.mkv"
}
```

### Response Format
```json
{
  "id": "uuid-string",
  "ok": true,
  "type": "DaemonInfo",
  "payload": {
    "port": 54321,
    "token": "daemon-auth-token",
    "version": "0.2.0",
    "roots": [{ "key": "...", "path": "...", "display_name": "...", ... }]
  }
}
```

### Event Format (unsolicited, pushed by system-bridge)
```json
{
  "event": "MagnetAdded",
  "payload": { "link": "magnet:?xt=urn:btih:..." }
}
```

```json
{
  "event": "TorrentAdded",
  "payload": { "name": "...", "infohash": "...", "contentsBase64": "..." }
}
```

Events have no `id` field — that's how the Rust backend distinguishes them from responses.

## KV Storage

### Chrome Extension (contexts 1–3, 5)

Routes through the Service Worker to `chrome.storage.local` (desktop) or Android SQLite (ChromeOS ARC).

Supports `area: 'sync'` for settings that sync across devices via `chrome.storage.sync`.

Key prefixes: `config:` for settings, `session:` for engine state, empty for credentials.

### Tauri (context 4)

Uses `localStorage` directly. Simple and sufficient — there's no sync storage in Tauri, and localStorage persists across sessions.

Key format: `jst:{prefix}{key}` (e.g., `jst:config:theme`, `jst:session:torrent:abc123`)

`area: 'sync'` is silently treated as `'local'`.

### Session Store

`HostChannelSessionStore` implements `ISessionStore` (from `@jstorrent/engine`) using `HostChannel.kvGet/kvSet`. Replaces `ExternalChromeStorageSessionStore` in the client package. Binary values are base64-encoded for storage.

### Config Hub

`HostChannelConfigHub` extends `BaseConfigHub` (from `@jstorrent/engine`) using `HostChannel.kvGet/kvSet` with `keyPrefix: 'config:'`. Replaces `ChromeConfigHub` in the client package.

## System-Bridge Changes

**None required for Phase 1.** The system-bridge works as-is when spawned by Tauri. Key behaviors:

- Parent process detection: walks process tree looking for a browser. When launched by Tauri, finds no known browser — falls back to the Tauri process as the parent. This is fine; the browser info in `rpc-info.json` is metadata, not functional.
- Extension ID from args: Chrome passes `chrome-extension://<id>/` as argv. Tauri doesn't — the extension ID comes via the Handshake message instead (field: `extensionId`).
- Stdin EOF = shutdown: when the Tauri app exits, the stdin pipe closes, system-bridge reads EOF and exits, which kills io-daemon (parent-pid monitoring).

**Potential future changes:**
- Accept `--mode tauri` flag to skip browser detection (minor optimization)
- Accept `--install-id` as CLI arg to avoid requiring Handshake (simplification)

## Client Package Refactoring

### New Files

| File | Purpose |
|------|---------|
| `packages/client/src/host/host-channel.ts` | Interface definition |
| `packages/client/src/host/types.ts` | Shared types (HostState, KVOpts, etc.) |
| `packages/client/src/host/chrome-extension-channel.ts` | Chrome extension implementation |
| `packages/client/src/host/tauri-channel.ts` | Tauri implementation |
| `packages/client/src/host/create-host-channel.ts` | Factory (detects context, returns impl) |
| `packages/client/src/host/HostChannelContext.tsx` | React context provider + `useHostChannel()` |
| `packages/client/src/host/host-channel-session-store.ts` | ISessionStore adapter |
| `packages/client/src/host/host-channel-config-hub.ts` | ConfigHub adapter |
| `packages/client/src/host/index.ts` | Re-exports |

### Modified Files

| File | Change |
|------|--------|
| `hooks/useIOBridgeState.ts` | Use `channel.onStateChanged` / `channel.onEvent` instead of Chrome port |
| `hooks/useSystemBridge.ts` | Use `channel.getVersion()` instead of `chrome.runtime.getManifest()` |
| `hooks/useChromeOSBootstrap.ts` | Type-narrow to `ChromeExtensionChannel` for ChromeOS methods |
| `engine-manager/chrome-extension-engine-manager.ts` | Accept HostChannel, replace all `sendMessage`/`postMessage` calls |
| `config/chrome-config-hub.ts` | Replace with or delegate to `HostChannelConfigHub` |
| `chrome/notification-bridge.ts` | Accept HostChannel, use `channel.notify()` |
| `components/SettingsOverlay.tsx` | Use `channel.clearSessionStorage()`, `channel.requestPermission()` |
| `App.tsx` | Create HostChannel, wrap in `HostChannelProvider`, create engine manager with channel |

### Deleted Files

| File | Reason |
|------|--------|
| `chrome/extension-bridge.ts` | Replaced by `host/chrome-extension-channel.ts` + factory |

## What Tauri Provides

| Capability | Notes |
|---|---|
| Code signing + notarization | macOS, Windows — handles sidecar binaries too |
| Auto-update | Built-in updater plugin |
| System tray / menu bar | Already implemented |
| Native webview | WKWebView (macOS), WebView2 (Windows) — no bundled Chromium |
| Installer generation | dmg, msi, deb, AppImage |
| Deep link / protocol handler | `tauri-plugin-deep-link` for `magnet:` URIs |
| Future native features | File dialogs, notifications, clipboard — available when needed |

## What Tauri Delegates to System-Bridge

| Capability | Why |
|---|---|
| File picker dialog | Already works cross-platform in system-bridge |
| Root management | Already persisted in `rpc-info.json` by system-bridge |
| File open / reveal | Already implemented with path safety validation |
| io-daemon lifecycle | System-bridge spawns, monitors, and cleans up io-daemon |

This keeps the Tauri Rust backend thin (message relay + system tray + deep links) and avoids duplicating tested logic.

## Implementation Phases

All file paths below are relative to `packages/client/src/` unless otherwise noted.

### Phase 1: HostChannel Interface + Types

Define the interface and supporting types. No behavioral changes. No existing files modified.

**New files:**

1. **`host/types.ts`** — All shared types: `HostState`, `HostCapabilities`, `KVOpts`, `HostNotification`, `NativeEvent`, `Unsubscribe`, `ConnectionStatus`, `Platform`. Re-export `DaemonInfo`, `DownloadRoot`, `DaemonStats` from their current locations (or define them here if they're currently inline).

2. **`host/host-channel.ts`** — The `HostChannel` interface as specified in the "HostChannel Interface" section above. Import types from `./types`. Export only the interface.

3. **`host/index.ts`** — Re-export everything from `types.ts` and `host-channel.ts`.

**Existing types to consolidate:** `DaemonBridgeState` (in `useIOBridgeState.ts`), `DaemonInfo` / `DownloadRoot` (in `useIOBridgeState.ts`), `DaemonStats` (in `useIOBridgeState.ts`). These are currently defined inline in the hook file. Move the type definitions into `host/types.ts` and have the hook import from there. This is a safe refactor — types only, no runtime change.

**Gate:** `pnpm run typecheck` passes. No runtime behavior to test.

### Phase 2: ChromeExtensionChannel

Implement `ChromeExtensionChannel` that wraps all Chrome extension messaging into the HostChannel interface. This is a new file — no existing files are modified yet.

**New file: `host/chrome-extension-channel.ts`**

The class has two modes controlled by the constructor:
- **Internal mode** (no extensionId): `chrome.runtime.sendMessage(message, callback)` — used inside the extension
- **External mode** (with extensionId): `chrome.runtime.sendMessage(extensionId, message, callback)` — used from website / dev server

```typescript
class ChromeExtensionChannel implements HostChannel {
  private extensionId: string | null
  private port: chrome.runtime.Port | null = null
  private stateListeners = new Set<(state: HostState) => void>()
  private eventListeners = new Set<(event: NativeEvent) => void>()
  private currentState: HostState = { status: 'connecting', ... }
  private reconnectTimeout: ReturnType<typeof setTimeout> | null = null

  constructor(extensionId?: string) {
    this.extensionId = extensionId ?? null
  }
}
```

**Key implementation details per method:**

`connect()`:
1. Send `GET_BRIDGE_STATE` via sendMessage → initialize `currentState`
2. Open port via `chrome.runtime.connect({name:'ui'})` (or with extensionId for external)
3. Attach port.onMessage listener that handles:
   - `BRIDGE_STATE_CHANGED` → update `currentState`, notify `stateListeners`
   - `CHROMEOS_BOOTSTRAP_STATE` → stored for ChromeOS-specific methods
   - `CLOSE` → `window.close()`
   - Messages with `event` field → notify `eventListeners`
4. Attach port.onDisconnect listener with visibility-based auto-reconnect:
   - If `document.visibilityState === 'visible'`: reconnect after 100ms
   - Else: wait for `visibilitychange` event to reconnect
   - Set `portStatus` to `'disconnected'` while disconnected

This is the port management logic currently at `useIOBridgeState.ts:171-215`. Extract it verbatim, adapting from React state to plain callbacks.

`onStateChanged(cb)` / `onEvent(cb)`:
- Add callback to the respective Set, return a function that removes it.

`kvGet(key, opts)`:
```typescript
const response = await this.sendMessage({
  type: 'KV_GET',
  key,
  keyPrefix: opts?.keyPrefix ?? 'session:',
  area: opts?.area ?? 'local',
})
return response.ok ? response.value : undefined
```

Same pattern for `kvGetMulti`, `kvSet`, `kvDelete`, `kvKeys`, `kvClear` — each maps to the corresponding `KV_*` message type. The `keyPrefix` default is `'session:'` to match existing behavior.

`pickDownloadFolder()`:
```typescript
const response = await this.sendMessage({ type: 'PICK_DOWNLOAD_FOLDER' })
return response.ok ? response.root : null
```

`removeDownloadRoot(key)`:
```typescript
await this.sendMessage({ type: 'REMOVE_DOWNLOAD_ROOT', key })
```

`openFile(rootKey, path)`:
```typescript
await this.sendMessage({ type: 'OPEN_FILE', rootKey, path })
```

`revealInFolder(rootKey, path)`:
```typescript
await this.sendMessage({ type: 'REVEAL_IN_FOLDER', rootKey, path })
```

`notify(notification)`:
```typescript
// Fire-and-forget via postMessage (port or sendMessage)
this.postMessage({ type: 'notification:' + notification.type, ...notification })
```

`retryConnection()`:
```typescript
this.reconnectPort()
this.postMessage({ type: 'RETRY_CONNECTION' })
```

`triggerLaunch()`:
```typescript
this.postMessage({ type: 'TRIGGER_LAUNCH' })
```

`getStats()`:
```typescript
const response = await this.sendMessage({ type: 'GET_DAEMON_STATS' })
return response.ok ? response.stats : null
```

`getDaemonInfo()`:
```typescript
const response = await this.sendMessage({ type: 'GET_DAEMON_INFO' })
return response.ok ? response.daemonInfo : null
// Also return roots from this response
```

`clearSessionStorage()`:
```typescript
await this.sendMessage({ type: 'CLEAR_SESSION_STORAGE' })
```

`notifyClosing()`:
```typescript
this.postMessage({ type: 'UI_CLOSING' })
```

`getVersion()`:
```typescript
return chrome.runtime.getManifest?.()?.version ?? null
```

`isDevMode()`:
```typescript
return !chrome.runtime.getManifest?.()?.update_url
```

`requestPermission(permission)`:
```typescript
if (typeof chrome !== 'undefined' && chrome.permissions?.request) {
  return chrome.permissions.request({ permissions: [permission] })
}
return true // No permission system
```

`capabilities`:
```typescript
get capabilities(): HostCapabilities {
  return {
    rootsManageable: true,
    hasSync: true,
    hasNativeNotifications: true,
    hasBackgroundPersistence: false, // SW may suspend
  }
}
```

**Private helpers:**

`sendMessage<T>(message)` — Promise wrapper around `chrome.runtime.sendMessage`, handling extensionId and `chrome.runtime.lastError`. This is the same pattern as the current `sendKVMessage` in `chrome-config-hub.ts:57-83`.

`postMessage(message)` — Fire-and-forget via port if connected, else `chrome.runtime.sendMessage` ignoring the response.

`reconnectPort()` — Disconnect existing port, open new one, re-attach listeners.

**ChromeOS-specific methods** (on the concrete class, not on HostChannel):

```typescript
openChromeOSIntent(): void {
  this.postMessage({ type: 'CHROMEOS_OPEN_INTENT' })
}

resetChromeOSPairing(): void {
  this.postMessage({ type: 'CHROMEOS_RESET_PAIRING' })
}

onChromeOSBootstrapStateChanged(cb): Unsubscribe { ... }
```

**Extension ID discovery logic** — migrated from `extension-bridge.ts:139-172`:
- Check `import.meta.env.DEV_EXTENSION_ID`
- Check `localStorage` for `jstorrent_extension_id`
- Check URL query param `?extensionId=`
- Fall back to published ID `dbokmlpefliilbjldladbimlcfgbolhk`

This lives in `create-host-channel.ts` (Phase 3) or as a static method on ChromeExtensionChannel.

**Gate:** `pnpm run typecheck` + `pnpm run test`. Write unit tests mocking `chrome.runtime`.

### Phase 3: Migrate Client Callers

Wire everything together: factory, context, adapters. Migrate all callers from `getBridge()` / direct `chrome.runtime` to HostChannel. Delete `extension-bridge.ts`.

**This is the highest-risk phase.** All existing Chrome extension behavior must be preserved.

**New files:**

1. **`host/create-host-channel.ts`** — Factory function:
   ```typescript
   export function createHostChannel(): HostChannel {
     if (isTauriContext()) {
       // Dynamic import to avoid loading Tauri code in Chrome
       const { TauriChannel } = require('./tauri-channel')
       return new TauriChannel()
     }
     if (isExtensionContext()) {
       return new ChromeExtensionChannel() // internal mode
     }
     // External (website / dev server)
     const extensionId = getExtensionId() // migrated from extension-bridge.ts
     return new ChromeExtensionChannel(extensionId)
   }
   ```
   Since TauriChannel doesn't exist yet in Phase 3, the Tauri branch can throw or return a stub. It gets implemented in Phase 5.

2. **`host/HostChannelContext.tsx`** — React context:
   ```typescript
   const HostChannelCtx = createContext<HostChannel | null>(null)

   export function HostChannelProvider({ channel, children }) {
     return <HostChannelCtx.Provider value={channel}>{children}</HostChannelCtx.Provider>
   }

   export function useHostChannel(): HostChannel {
     const ctx = useContext(HostChannelCtx)
     if (!ctx) throw new Error('useHostChannel must be used within HostChannelProvider')
     return ctx
   }
   ```

3. **`host/host-channel-session-store.ts`** — ISessionStore adapter:
   ```typescript
   export class HostChannelSessionStore implements ISessionStore {
     constructor(private channel: HostChannel) {}

     async get(key: string): Promise<Uint8Array | null> {
       const value = await this.channel.kvGet<string>(key, { keyPrefix: 'session:' })
       return value ? base64ToUint8Array(value) : null
     }

     async set(key: string, value: Uint8Array): Promise<void> {
       await this.channel.kvSet(key, uint8ArrayToBase64(value), { keyPrefix: 'session:' })
     }

     async delete(key: string): Promise<void> {
       await this.channel.kvDelete(key, { keyPrefix: 'session:' })
     }

     async keys(prefix?: string): Promise<string[]> {
       return this.channel.kvKeys(prefix, { keyPrefix: 'session:' })
     }

     async clear(): Promise<void> {
       await this.channel.kvClear(undefined, { keyPrefix: 'session:' })
     }

     async getMulti(keys: string[]): Promise<Map<string, Uint8Array>> {
       const raw = await this.channel.kvGetMulti(keys, { keyPrefix: 'session:' })
       const result = new Map<string, Uint8Array>()
       for (const [k, v] of Object.entries(raw)) {
         if (v) result.set(k, base64ToUint8Array(v as string))
       }
       return result
     }

     async getJson<T>(key: string): Promise<T | null> {
       const value = await this.channel.kvGet<T>(key, { keyPrefix: 'session:' })
       return value ?? null
     }

     async setJson<T>(key: string, value: T): Promise<void> {
       await this.channel.kvSet(key, value, { keyPrefix: 'session:' })
     }
   }
   ```

4. **`host/host-channel-config-hub.ts`** — ConfigHub adapter. Same structure as existing `ChromeConfigHub` but uses `channel.kvGetMulti` / `channel.kvSet` instead of `sendKVMessage`. The sync vs local storage area mapping stays the same — the `area` field is passed through to the channel.

**Modified files — exact changes for each:**

5. **`App.tsx`** — Create HostChannel at startup, wrap in provider:
   ```typescript
   // Replace:
   //   const isDevBuild = chrome.runtime.getManifest?.()?.update_url === undefined
   // With:
   //   const channel = useMemo(() => createHostChannel(), [])
   //   const isDevBuild = channel.isDevMode()
   //
   // Add useEffect to connect/disconnect:
   //   useEffect(() => { channel.connect(); return () => channel.disconnect() }, [channel])
   //
   // Wrap render tree in:
   //   <HostChannelProvider channel={channel}>...</HostChannelProvider>
   //
   // Replace engineManager singleton creation:
   //   Currently: module-level `const engineManager = new ChromeExtensionEngineManager()`
   //   After: created inside App with `channel` dependency, passed via EngineManagerProvider
   ```

6. **`hooks/useIOBridgeState.ts`** — Rewrite to use HostChannel subscriptions. This removes ~200 lines of port management code:
   ```typescript
   export function useIOBridgeState(config = {}): UseIOBridgeStateResult {
     const channel = useHostChannel()
     const [state, setState] = useState<HostState>(channel.getState())
     const [hasEverConnected, setHasEverConnected] = useState(false)

     useEffect(() => {
       // Sync initial state
       setState(channel.getState())
       if (channel.getState().status === 'connected') setHasEverConnected(true)

       const unsubState = channel.onStateChanged((newState) => {
         setState(newState)
         if (newState.status === 'connected') setHasEverConnected(true)
       })
       const unsubEvent = channel.onEvent((event) => {
         config.onNativeEvent?.(event.event, event.payload)
       })
       return () => { unsubState(); unsubEvent() }
     }, [channel])

     const retry = useCallback(() => channel.retryConnection(), [channel])
     const launch = useCallback(() => channel.triggerLaunch(), [channel])
     const getStats = useCallback(() => channel.getStats(), [channel])

     return {
       state,
       isConnected: state.status === 'connected',
       hasEverConnected,
       retry, launch,
       cancel: () => {}, // no-op, kept for API compat
       getStats,
       // ChromeOS bootstrap: handled separately via type narrowing
       chromeosBootstrapState: null, // moved to useChromeOSBootstrap
       chromeosHasEverConnected: false,
       portStatus: state.status === 'connected' ? 'connected' : 'disconnected',
     }
   }
   ```
   Note: `chromeosBootstrapState` moves to `useChromeOSBootstrap` which now subscribes directly via `ChromeExtensionChannel.onChromeOSBootstrapStateChanged()`.

7. **`hooks/useChromeOSBootstrap.ts`** — Use HostChannel from context, type-narrow for ChromeOS:
   ```typescript
   const channel = useHostChannel()
   // Type-narrow: ChromeOS methods only exist on ChromeExtensionChannel
   const openIntent = useCallback(() => {
     if ('openChromeOSIntent' in channel) {
       (channel as ChromeExtensionChannel).openChromeOSIntent()
     }
   }, [channel])
   ```

8. **`hooks/useSystemBridge.ts`** — Replace `chrome.runtime.getManifest().version`:
   ```typescript
   // Replace:  chrome.runtime.getManifest().version
   // With:     channel.getVersion() ?? 'unknown'
   ```

9. **`engine-manager/chrome-extension-engine-manager.ts`** — Accept HostChannel, replace all bridge calls:
   - Constructor takes `HostChannel` parameter
   - Delete `sendKVMessage` helper function (replaced by `channel.kvGet/kvSet`)
   - Replace `getBridge().sendMessage({type:'GET_DAEMON_INFO'})` → `channel.getDaemonInfo()`
   - Replace `getBridge().sendMessage({type:'PICK_DOWNLOAD_FOLDER'})` → `channel.pickDownloadFolder()`
   - Replace `getBridge().sendMessage({type:'OPEN_FILE', rootKey, path})` → `channel.openFile(rootKey, path)`
   - Replace `getBridge().sendMessage({type:'REVEAL_IN_FOLDER', rootKey, path})` → `channel.revealInFolder(rootKey, path)`
   - Replace `getBridge().sendMessage({type:'REMOVE_DOWNLOAD_ROOT', key})` → `channel.removeDownloadRoot(key)`
   - Replace `getBridge().postMessage({type:'UI_CLOSING'})` → `channel.notifyClosing()`
   - Replace `getBridge().postMessage({type:'IOBRIDGE_AUTH_FAILED'})` → `channel.retryConnection()` (or add `notifyAuthFailed()` to HostChannel)
   - Replace `new ExternalChromeStorageSessionStore(extensionId)` → `new HostChannelSessionStore(channel)`
   - Replace `new ChromeConfigHub(extensionId)` → `new HostChannelConfigHub(channel)`
   - Replace credentials getter `sendKVMessage(extensionId, {type:'KV_GET', key:'android:authToken'})` → `channel.kvGet('android:authToken', {keyPrefix:'', area:'local'})`
   - Delete the `connectToServiceWorker()` method — port management is now in ChromeExtensionChannel
   - Remove import of `getBridge`

10. **`config/chrome-config-hub.ts`** — Replace with `HostChannelConfigHub` or refactor to accept HostChannel:
    - Delete `sendKVMessage` helper
    - Replace `sendKVMessage(extensionId, {type:'KV_GET_MULTI', keys, keyPrefix:'', area})` → `channel.kvGetMulti(keys, {keyPrefix:'', area})`
    - Replace `sendKVMessage(extensionId, {type:'KV_SET', key, value, keyPrefix:'', area})` → `channel.kvSet(key, value, {keyPrefix:'', area})`
    - Constructor takes `HostChannel` instead of `extensionId`

11. **`chrome/notification-bridge.ts`** — Accept HostChannel, replace `getBridge()`:
    - Constructor or `init()` takes `HostChannel`
    - Replace all `getBridge().postMessage({type:'notification:visibility', ...})` → `channel.notify({type:'visibility', ...})`
    - Replace all `getBridge().postMessage({type:'notification:stats', ...})` → `channel.notify({type:'stats', ...})`
    - Replace all `getBridge().postMessage({type:'notification:torrent-complete', ...})` → `channel.notify({type:'torrent-complete', ...})`
    - Same for `torrent-error` and `duplicate-torrent`
    - The singleton export changes: `createNotificationBridge(channel)` factory instead of module-level instance

12. **`components/SettingsOverlay.tsx`** — Use HostChannel from context:
    - Replace `chrome.runtime.sendMessage({type:'CLEAR_SESSION_STORAGE'})` → `channel.clearSessionStorage()`
    - Replace `chrome.permissions.request({permissions:['power']})` → `channel.requestPermission('power')`
    - Get channel via `useHostChannel()` hook

**Deleted files:**

13. **`chrome/extension-bridge.ts`** — All functionality moved to `host/chrome-extension-channel.ts` + `host/create-host-channel.ts`. Remove all imports of `getBridge`, `createBridge`, `ExtensionBridge` from other files.

**Update exports:**

14. **`index.ts`** — Remove `getBridge`/`ExtensionBridge` exports. Add `createHostChannel`, `HostChannel`, `HostChannelProvider`, `useHostChannel` exports.

**Gate:** `pnpm run typecheck` + `pnpm run test` + **extension E2E**. Manual smoke test: load extension → "Connected" → add magnet → settings persist.

### Phase 4: Tauri Rust Backend

Rewrite `desktop/tauri-app/src-tauri/src/lib.rs` to spawn system-bridge instead of io-daemon. Implement native messaging relay.

**Modified files:**

1. **`desktop/tauri-app/src-tauri/src/lib.rs`** — Full rewrite of sidecar management:

   **Remove:** `DaemonInfo` struct, `DaemonState` struct, `get_daemon_info` command, direct io-daemon spawning code.

   **Add:** `HostBridge` struct that owns the stdin writer and pending request map:

   ```rust
   use std::collections::HashMap;
   use tokio::sync::{oneshot, Mutex};
   use tokio::process::{Child, ChildStdin};
   use tokio::io::AsyncWriteExt;

   struct HostBridge {
       stdin: Mutex<ChildStdin>,
       pending: Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>,
   }

   impl HostBridge {
       /// Write a length-prefixed JSON message to system-bridge stdin.
       async fn send(&self, message: &serde_json::Value) -> Result<(), String> {
           let json = serde_json::to_vec(message).map_err(|e| e.to_string())?;
           let len = (json.len() as u32).to_le_bytes();
           let mut stdin = self.stdin.lock().await;
           stdin.write_all(&len).await.map_err(|e| e.to_string())?;
           stdin.write_all(&json).await.map_err(|e| e.to_string())?;
           stdin.flush().await.map_err(|e| e.to_string())?;
           Ok(())
       }

       /// Send a request and wait for the matching response.
       async fn request(&self, message: serde_json::Value) -> Result<serde_json::Value, String> {
           let id = uuid::Uuid::new_v4().to_string();
           let mut msg = message;
           msg.as_object_mut().unwrap().insert("id".into(), id.clone().into());

           let (tx, rx) = oneshot::channel();
           self.pending.lock().await.insert(id, tx);

           self.send(&msg).await?;
           rx.await.map_err(|_| "Channel closed".to_string())
       }
   }
   ```

   **Background stdout reader task** — spawned during `setup()`:
   ```rust
   // Spawn tokio task to read system-bridge stdout
   tauri::async_runtime::spawn(async move {
       let mut stdout = child_stdout; // BufReader<ChildStdout>
       loop {
           // Read 4-byte length prefix
           let mut len_buf = [0u8; 4];
           if stdout.read_exact(&mut len_buf).await.is_err() {
               break; // EOF or error
           }
           let len = u32::from_le_bytes(len_buf) as usize;
           let mut buf = vec![0u8; len];
           if stdout.read_exact(&mut buf).await.is_err() {
               break;
           }

           let msg: serde_json::Value = match serde_json::from_slice(&buf) {
               Ok(v) => v,
               Err(_) => continue,
           };

           // Dispatch: response (has "id") vs event (has "event")
           if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
               // Response — resolve pending request
               if let Some(tx) = bridge.pending.lock().await.remove(id) {
                   let _ = tx.send(msg);
               }
           } else if msg.get("event").is_some() {
               // Event — emit to frontend
               let _ = app_handle.emit("host-event", &msg);
           }
       }
   });
   ```

   **Tauri commands:**
   ```rust
   #[tauri::command]
   async fn host_handshake(
       state: tauri::State<'_, Arc<HostBridge>>,
   ) -> Result<serde_json::Value, String> {
       let install_id = get_or_create_install_id(); // Load from app data dir, or create + persist
       state.request(serde_json::json!({
           "op": "handshake",
           "extensionId": "tauri-desktop",
           "installId": install_id,
       })).await
   }

   #[tauri::command]
   async fn host_message(
       state: tauri::State<'_, Arc<HostBridge>>,
       message: serde_json::Value,
   ) -> Result<serde_json::Value, String> {
       state.request(message).await
   }
   ```

   **Install ID persistence:** Store in Tauri's app data directory (`app.path().app_data_dir()`) as a plain text file `install-id`. Create on first launch, reuse thereafter.

   **Sidecar spawning:** System-bridge binary name is `jstorrent-host` (matching the existing binary name). Find io-daemon binary in the same directory.

   ```rust
   let sidecar = shell.sidecar("binaries/jstorrent-host")
       .expect("failed to create sidecar command");
   // No args needed — system-bridge reads extension ID and install ID from Handshake message.
   // Chrome passes chrome-extension://... as argv[1], but system-bridge doesn't require it.
   ```

   **System tray:** Unchanged from current implementation.

   **Window close handling:** Unchanged (hide on close, quit via tray menu).

   **invoke_handler:** Replace `get_daemon_info` with `host_handshake` and `host_message`:
   ```rust
   .invoke_handler(tauri::generate_handler![host_handshake, host_message])
   ```

2. **`desktop/tauri-app/src-tauri/Cargo.toml`** — May need `dashmap` crate for concurrent pending map, or use `tokio::sync::Mutex<HashMap>`. Add `byteorder` if not using manual `to_le_bytes()` (the stdlib method is fine).

3. **`desktop/tauri-app/src-tauri/capabilities/default.json`** — Already has `shell:allow-spawn` and `shell:allow-execute`. No changes needed.

4. **`desktop/tauri-app/scripts/prepare-sidecar.sh`** — Build and copy both binaries:
   ```bash
   # Build system-bridge (native host)
   cargo build --release -p jstorrent-host
   cp "$TARGET/release/jstorrent-host" src-tauri/binaries/jstorrent-host-$TRIPLE

   # Build io-daemon
   cargo build --release -p jstorrent-io-daemon
   cp "$TARGET/release/jstorrent-io-daemon" src-tauri/binaries/jstorrent-io-daemon-$TRIPLE

   # Dev copies (without triple suffix)
   mkdir -p "$TARGET/debug/binaries"
   cp "$TARGET/release/jstorrent-host" "$TARGET/debug/binaries/jstorrent-host"
   cp "$TARGET/release/jstorrent-io-daemon" "$TARGET/debug/binaries/jstorrent-io-daemon"
   ```

5. **`desktop/tauri-app/src-tauri/tauri.conf.json`** — Update externalBin:
   ```json
   "externalBin": [
     "binaries/jstorrent-host",
     "binaries/jstorrent-io-daemon"
   ]
   ```

**Gate:** Rust integration test (see Validation Strategy section). Also: `cargo build` succeeds, `pnpm tauri dev` launches without crashing.

### Phase 5: TauriChannel

Implement the Tauri frontend channel.

**New file: `host/tauri-channel.ts`**

```typescript
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { HostChannel, HostState, HostCapabilities, ... } from './types'

export class TauriChannel implements HostChannel {
  private currentState: HostState = {
    status: 'connecting', platform: 'tauri',
    daemonInfo: null, roots: [], lastError: null,
  }
  private stateListeners = new Set<(state: HostState) => void>()
  private eventListeners = new Set<(event: NativeEvent) => void>()
  private eventUnlisten: UnlistenFn | null = null
  private daemonInfo: { port: number; token: string } | null = null
}
```

`connect()`:
```typescript
async connect(): Promise<void> {
  try {
    // 1. Send handshake to system-bridge via Tauri backend
    const response = await invoke<any>('host_handshake')

    if (response.ok && response.type === 'DaemonInfo') {
      const { port, token, version, roots } = response.payload
      this.daemonInfo = { port, token }
      this.updateState({
        status: 'connected',
        platform: 'tauri',
        daemonInfo: { port, token, version, roots, host: '127.0.0.1' },
        roots,
        lastError: null,
      })
    } else {
      this.updateState({
        ...this.currentState,
        status: 'disconnected',
        lastError: response.error ?? 'Handshake failed',
      })
    }

    // 2. Listen for events from system-bridge (MagnetAdded, TorrentAdded)
    this.eventUnlisten = await listen('host-event', (event) => {
      const payload = event.payload as any
      if (payload.event) {
        for (const cb of this.eventListeners) {
          cb({ event: payload.event, payload: payload.payload })
        }
      }
    })
  } catch (e) {
    this.updateState({
      ...this.currentState,
      status: 'disconnected',
      lastError: String(e),
    })
  }
}
```

`kvGet(key, opts)`:
```typescript
const prefixed = (opts?.keyPrefix ?? 'session:') + key
const raw = localStorage.getItem(`jst:${prefixed}`)
return raw != null ? JSON.parse(raw) : undefined
```

`kvGetMulti(keys, opts)`:
```typescript
const prefix = opts?.keyPrefix ?? 'session:'
const result: Record<string, unknown> = {}
for (const key of keys) {
  const raw = localStorage.getItem(`jst:${prefix}${key}`)
  if (raw != null) result[key] = JSON.parse(raw)
}
return result
```

`kvSet(key, value, opts)`:
```typescript
const prefixed = (opts?.keyPrefix ?? 'session:') + key
localStorage.setItem(`jst:${prefixed}`, JSON.stringify(value))
```

`kvDelete`, `kvKeys`, `kvClear` — similar localStorage operations.

`pickDownloadFolder()`:
```typescript
const response = await invoke<any>('host_message', {
  message: { op: 'pickDownloadDirectory' }
})
if (response.ok && response.type === 'RootAdded') {
  // Update local state with new root
  const newRoots = [...this.currentState.roots, response.payload.root]
  this.updateState({ ...this.currentState, roots: newRoots })
  return response.payload.root
}
return null
```

`openFile(rootKey, path)`:
```typescript
await invoke('host_message', {
  message: { op: 'openFile', rootKey, path }
})
```

`revealInFolder(rootKey, path)`:
```typescript
await invoke('host_message', {
  message: { op: 'revealInFolder', rootKey, path }
})
```

`removeDownloadRoot(key)`:
```typescript
await invoke('host_message', {
  message: { op: 'deleteDownloadRoot', key }
})
const newRoots = this.currentState.roots.filter(r => r.key !== key)
this.updateState({ ...this.currentState, roots: newRoots })
```

`getStats()`:
```typescript
if (!this.daemonInfo) return null
const { port, token } = this.daemonInfo
const response = await fetch(`http://127.0.0.1:${port}/stats`, {
  headers: { 'X-JST-Auth': token },
})
return response.ok ? response.json() : null
```

`notify()`: No-op initially.

`retryConnection()`: Re-run `connect()`.

`triggerLaunch()`: No-op (daemon is always launched by system-bridge).

`clearSessionStorage()`:
```typescript
const keysToRemove: string[] = []
for (let i = 0; i < localStorage.length; i++) {
  const key = localStorage.key(i)
  if (key?.startsWith('jst:session:')) keysToRemove.push(key)
}
keysToRemove.forEach(k => localStorage.removeItem(k))
```

`getVersion()`: Return build-time version from `import.meta.env.PACKAGE_VERSION` (set in vite.config.ts via `define`), or call `invoke('get_version')`.

`isDevMode()`: Return `import.meta.env.DEV`.

`requestPermission()`: Return `true` (desktop apps have full permissions).

`capabilities`:
```typescript
get capabilities(): HostCapabilities {
  return {
    rootsManageable: true,
    hasSync: false,
    hasNativeNotifications: false,
    hasBackgroundPersistence: true,
  }
}
```

**Also update `host/create-host-channel.ts`** — replace the Tauri stub/throw with actual TauriChannel instantiation.

**Gate:** Manual smoke test — `pnpm tauri dev`, verify connected state, test pick folder, test settings persistence.

### Phase 6: Deep Links

Register Tauri as protocol handler for `magnet:` URIs.

**Modified files:**

1. **`desktop/tauri-app/src-tauri/Cargo.toml`** — Add `tauri-plugin-deep-link` dependency.

2. **`desktop/tauri-app/src-tauri/src/lib.rs`** — Register deep-link handler:
   ```rust
   .plugin(tauri_plugin_deep_link::init())
   .setup(move |app| {
       // ... existing setup ...

       // Register deep link handler
       app.deep_link().on_open_url(|event| {
           for url in event.urls() {
               let url_str = url.to_string();
               if url_str.starts_with("magnet:") {
                   // Forward to system-bridge stdin as a magnet event
                   // Or: POST to system-bridge RPC server
                   // The system-bridge will push MagnetAdded on stdout
               }
           }
       });
   })
   ```

3. **`desktop/tauri-app/src-tauri/capabilities/default.json`** — Add deep-link permission.

4. **`desktop/tauri-app/src-tauri/tauri.conf.json`** — Register protocol associations:
   ```json
   "bundle": {
     "fileAssociations": [
       { "ext": ["torrent"], "mimeType": "application/x-bittorrent" }
     ]
   }
   ```

**Gate:** Manual test — `open "magnet:?xt=urn:btih:..."` from terminal, verify arrival in UI.

### Future Phases

- **Notifications**: Wire up `tauri-plugin-notification` in TauriChannel
- **Auto-update**: Configure Tauri updater plugin
- **Adopt Tauri file dialogs**: Optionally replace system-bridge's file picker with Tauri's native dialog (removes one reason to ship system-bridge, but low priority)

## Validation Strategy

Each phase has a gate that must pass before proceeding.

### Phase 1: HostChannel Interface + Types

**Gate:** `pnpm run typecheck` passes across all packages. No runtime behavior to test.

### Phase 2: ChromeExtensionChannel

**Gate:** `pnpm run typecheck` + `pnpm run test`.

Unit tests for `ChromeExtensionChannel` mocking `chrome.runtime.sendMessage` and `chrome.runtime.connect`:
- Verify each HostChannel method sends the correct message type and shape
- Verify port reconnection logic (disconnect → visibility change → reconnect)
- Verify event/state callbacks fire correctly

### Phase 3: Migrate Client Callers

**Gate:** `pnpm run typecheck` + `pnpm run test` + **extension E2E**.

This is the highest-risk phase — all existing callers are rewired. The Chrome extension must work identically.

- Extension E2E tests validate no regression (daemon connects, torrents work, settings persist)
- Manual smoke test: load extension, verify "Connected" status, add a magnet, check settings persist across reload
- This phase should land as a single reviewable commit for easy bisect/revert

### Phase 4: Tauri Rust Backend

**Gate:** Rust integration test for the native messaging relay.

```rust
#[tokio::test]
async fn test_host_bridge_handshake() {
    // 1. Build system-bridge and io-daemon binaries
    // 2. Spawn system-bridge as child process
    // 3. Write Handshake message to stdin:
    //    4-byte LE length + JSON { id, op: "handshake", extensionId: "test", installId: "uuid" }
    // 4. Read response from stdout:
    //    4-byte LE length + JSON
    // 5. Assert: ok=true, type=DaemonInfo, port > 0, token is non-empty
    // 6. Send pickDownloadDirectory (will fail headless, but validates framing)
    // 7. Verify clean shutdown on stdin close (EOF)
}
```

This tests the critical path (spawn → handshake → daemon info) without any GUI or browser. It catches: binary-not-found, protocol framing bugs, handshake failures, daemon spawn failures.

Additionally: `cargo build` in src-tauri must succeed, and `pnpm tauri dev` must launch without crashing.

### Phase 5: TauriChannel

**Gate:** Tauri app renders with "Connected" state.

Manual smoke test:
1. `pnpm tauri dev` — app launches
2. Header shows connected status (not "Setup" or "Not connected")
3. Console shows HostChannel connected log, no errors
4. Settings page opens, theme toggle persists across reload (validates localStorage KV)
5. "Pick download folder" opens native dialog (validates system-bridge relay)

Optional automated test using Tauri's WebDriver support:
```rust
#[test]
fn test_tauri_app_smoke() {
    // 1. Launch Tauri app via WebDriver
    // 2. Wait for element with connected status text
    // 3. Assert no console errors containing "Error" or "reject"
    // 4. Close app
}
```

This is lower priority — the manual smoke test is fast and sufficient for initial development. The WebDriver test becomes valuable once we want CI validation.

### Phase 6: Deep Links

**Gate:** Manual test.

1. Register Tauri as `magnet:` protocol handler
2. Open a magnet link from a browser or terminal: `open "magnet:?xt=urn:btih:..."`
3. Verify Tauri app receives it and shows the "add torrent" UI
4. Verify `.torrent` file association works (double-click opens in Tauri)

### CI Integration

| Phase | CI Check | New? |
|---|---|---|
| 1–3 | `pnpm run typecheck && pnpm run test && pnpm run lint` | Existing |
| 3 | Extension E2E | Existing |
| 4 | `cargo build -p jstorrent-desktop` + Rust integration test | **New** |
| 4 | `cargo clippy` + `cargo fmt --check` for Tauri crate | **New** (mirrors system-bridge CI) |
| 5 | Tauri build succeeds: `pnpm tauri build` | Existing (system-bridge CI) |
| 5–6 | WebDriver smoke test (optional, future) | **New** |

The Rust integration test for Phase 4 is the most important new CI addition. It validates the Tauri ↔ system-bridge ↔ io-daemon chain without needing a display server, making it suitable for headless CI.

## Risk Assessment

| Risk | Mitigation |
|---|---|
| System-bridge expects Chrome parent process | Falls back gracefully — browser info is metadata only |
| System-bridge's `find_io_daemon_path` expects sibling binary | Both binaries ship in same Tauri sidecar directory — works by default |
| Module-level `engineManager` singleton depends on HostChannel | Move creation into `App` component, pass via existing `EngineManagerProvider` context |
| Chrome types (`chrome.runtime.Port`) loaded in Tauri | Dynamic imports in factory ensure Chrome-specific code never loads in Tauri |
| Tauri signing of extra binaries | `externalBin` in tauri.conf.json — Tauri signs all bundled binaries automatically |
