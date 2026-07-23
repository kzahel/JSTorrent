# Phase 1: Refactor — Rename upnp/ → port-mapping/

Pure rename and interface extraction. No new protocols, no behavior changes.
UPnP continues to work identically. One commit.

## Goals

1. Rename directory `upnp/` → `port-mapping/`
2. Extract `PortMappingProvider` interface from `GatewayDevice`
3. Rename `UPnPManager` → `PortMappingManager` (keep `UPnPManager` as type alias)
4. Move `NetworkInterface` to `interfaces/`
5. Update all imports across the codebase
6. Verify: `pnpm typecheck && pnpm test && pnpm lint`

## Step-by-Step

### 1. Move `NetworkInterface` to `interfaces/network.ts`

Create `packages/engine/src/interfaces/network.ts`:

```typescript
export interface NetworkInterface {
  name: string
  address: string
  prefixLength: number
}
```

Update `interfaces/index.ts` to re-export it.

**Why first:** Both `upnp-manager.ts` and future `gateway.ts` need it. Currently
defined in `upnp-manager.ts` which is awkward. Moving it before the rename keeps
the diff cleaner.

**Files that import `NetworkInterface` from upnp (must update):**
- `core/bt-engine.ts:21`
- `adapters/daemon/daemon-connection.ts:1`
- `presets/native.ts:20`
- `index.ts:115`

### 2. Rename directory

```bash
git mv packages/engine/src/upnp packages/engine/src/port-mapping
```

Files become:
```
packages/engine/src/port-mapping/
├── index.ts
├── upnp-manager.ts      → port-mapping-manager.ts  (step 4)
├── ssdp-client.ts        (unchanged)
└── gateway-device.ts     (unchanged)
```

### 3. Extract `PortMappingProvider` interface

Create in `port-mapping-manager.ts` (or a separate `types.ts` if preferred):

```typescript
export interface PortMappingProvider {
  /** Initialize / discover this provider. Returns true if usable. */
  init(): Promise<boolean>

  /** Add a port mapping. Returns true on success. */
  addPortMapping(
    externalPort: number,
    internalPort: number,
    internalClient: string,
    protocol: 'TCP' | 'UDP',
    description: string,
    leaseDuration: number,
  ): Promise<boolean>

  /** Remove a port mapping. Returns true on success. */
  deletePortMapping(
    externalPort: number,
    protocol: 'TCP' | 'UDP',
  ): Promise<boolean>

  /** Get external IP address. */
  getExternalIP(): Promise<string | null>

  /** External IP (cached after init). */
  readonly externalIP: string | null
}
```

`GatewayDevice` already has all these methods — make it `implements PortMappingProvider`.
The `init()` method maps to its existing `init()`. Future `NatPmpClient` and
`PcpClient` will also implement this interface.

### 4. Rename `UPnPManager` → `PortMappingManager`

In `port-mapping-manager.ts` (renamed from `upnp-manager.ts`):

```typescript
export class PortMappingManager {
  // ... same implementation
}

/** @deprecated Use PortMappingManager */
export type UPnPManager = PortMappingManager
export const UPnPManager = PortMappingManager
```

Keep the alias so external consumers (if any) aren't broken. The class internals
stay identical — it still only uses UPnP via `SSDPClient` + `GatewayDevice`.

Also rename `UPnPMapping`:

```typescript
export interface PortMapping {
  externalPort: number
  internalPort: number
  protocol: 'TCP' | 'UDP'
}

/** @deprecated Use PortMapping */
export type UPnPMapping = PortMapping
```

### 5. Update `port-mapping/index.ts`

```typescript
export { PortMappingManager, PortMappingManager as UPnPManager } from './port-mapping-manager'
export type { PortMappingProvider, PortMapping, PortMapping as UPnPMapping, } from './port-mapping-manager'
export { SSDPClient } from './ssdp-client'
export type { SSDPDevice } from './ssdp-client'
export { GatewayDevice } from './gateway-device'
```

### 6. Update imports across the codebase

**Engine core — `core/bt-engine.ts`:**
```diff
-import { UPnPManager, NetworkInterface } from '../upnp'
+import { PortMappingManager } from '../port-mapping'
+import type { NetworkInterface } from '../interfaces/network'
```

- Rename `private upnpManager?: UPnPManager` → `private portMappingManager?: PortMappingManager`
- Rename `enableUPnP()` → `enablePortMapping()`, `disableUPnP()` → `disablePortMapping()`
  (internal private methods, no external API breakage)
- Keep `upnpStatus` / `upnpExternalIP` getters as-is (public API, used by UI)

**Config — no changes needed:**
- `upnpEnabled`, `upnpStatus`, `UPnPStatus` — keep as-is, these are persisted
  config keys and user-facing names

**Adapters:**
```diff
# adapters/daemon/daemon-connection.ts
-import type { NetworkInterface } from '../../upnp/upnp-manager'
+import type { NetworkInterface } from '../../interfaces/network'

# presets/native.ts
-import type { NetworkInterface } from '../upnp/upnp-manager'
+import type { NetworkInterface } from '../interfaces/network'
```

**Main `index.ts`:**
```diff
-export { UPnPManager, SSDPClient, GatewayDevice } from './upnp'
-export type { NetworkInterface, UPnPMapping, SSDPDevice } from './upnp'
+export { PortMappingManager, PortMappingManager as UPnPManager, SSDPClient, GatewayDevice } from './port-mapping'
+export type { PortMappingProvider, PortMapping, PortMapping as UPnPMapping, SSDPDevice } from './port-mapping'
+export type { NetworkInterface } from './interfaces/network'
```

**Native controller (`adapters/native/controller.ts`):**
- No import changes — uses `engine.upnpStatus` / `engine.upnpExternalIP` which
  are still public getters on BtEngine

**Client/Extension:**
- `SettingsOverlay.tsx` imports `UPnPStatus` from `@jstorrent/engine` — no change
  needed (type still exported)
- `daemon-engine-manager.ts` passes `getNetworkInterfaces` — no change needed

**Android/Kotlin:**
- No changes — Kotlin code uses config keys (`upnpEnabled`) and JNI queries
  (`__jstorrent_query_upnp_status`) which don't reference TS module paths

### 7. Add `description` param to PortMappingManager

Currently hardcoded `'JSTorrent'` in `addMapping()` (line 112 of upnp-manager.ts).

```diff
 constructor(
   private socketFactory: ISocketFactory,
   private getNetworkInterfaces: () => Promise<NetworkInterface[]>,
   private logger?: Logger,
+  private description: string = 'JSTorrent',
 )
```

Use `this.description` instead of the hardcoded string. web-server will pass
`'200 OK Web Server'` when it copies this code.

### 8. Verify

```bash
pnpm run typecheck
pnpm run test
pnpm run lint
pnpm format:fix
```

All must pass with no behavior changes.

## Files Changed (summary)

| Action | File |
|--------|------|
| **Create** | `interfaces/network.ts` |
| **Edit** | `interfaces/index.ts` (add NetworkInterface re-export) |
| **Rename** | `upnp/` → `port-mapping/` |
| **Rename** | `upnp-manager.ts` → `port-mapping-manager.ts` |
| **Edit** | `port-mapping-manager.ts` (rename class, extract interface, add description param) |
| **Edit** | `port-mapping/index.ts` (update exports) |
| **Edit** | `port-mapping/gateway-device.ts` (implements PortMappingProvider) |
| **Edit** | `core/bt-engine.ts` (update imports, rename private methods/fields) |
| **Edit** | `adapters/daemon/daemon-connection.ts` (update import) |
| **Edit** | `presets/native.ts` (update import) |
| **Edit** | `index.ts` (update re-exports) |

## What NOT to Change

- Config keys: `upnpEnabled`, `upnpStatus` — persisted, backward compat
- `UPnPStatus` type — used across config layer, keep as canonical name
- Public getters: `upnpStatus`, `upnpExternalIP` — used by UI and native bindings
- Android/Kotlin code — doesn't reference TS paths
- Client components — import from `@jstorrent/engine`, types still exported
- `ssdp-client.ts` — unchanged
- `gateway-device.ts` — only adds `implements PortMappingProvider`
