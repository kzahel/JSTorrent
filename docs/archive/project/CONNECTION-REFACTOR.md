# Connection Architecture Refactor

Status: **Planned**

## Problem

The extension's connection system was built around two mutually exclusive paths: desktop native messaging and ChromeOS Android companion. When Crostini support was added (ChromeOS users running io-daemon in a Linux VM without Play Store), it was grafted on via hostname checks and scattered `isCrostini` conditionals. This works but is fragile and hard to extend.

See `docs/crostini-connection-fixes.md` for the full retrospective on the Crostini integration.

## Current Connection Configurations

| # | Config | Platform | Discovery | Connection | BackendType |
|---|--------|----------|-----------|------------|-------------|
| 1 | Desktop + native host | `desktop` | `chrome.runtime.connectNative` | Native messaging | `desktop` |
| 2 | Tauri standalone | `tauri` | Webview-internal | Self-hosted | `self` |
| 3 | ChromeOS + Android | `chromeos` | HTTP probe `100.115.92.2` | HTTP + WS | `android` |
| 4 | ChromeOS + Crostini | `chromeos` | HTTP probe `penguin.linux.test` | HTTP + WS | `desktop` (hacked) |
| 5 | Android standalone | N/A | N/A (in-process) | N/A | N/A |

## Issues

### 1. `BackendType` doesn't model reality

`'desktop' | 'android' | 'self'` conflates "Crostini standalone daemon" with "desktop native host + Tauri app." They have different capabilities (no folder picker, no auto-update, no `desktopVersion` field), yet share a backend type. The version check at `useSystemBridge.ts:149` papers over this: `state.daemonInfo.desktopVersion ?? undefined` returns `undefined` for Crostini, which passes as `'compatible'` — correct today but brittle.

**Files:** `packages/client/src/hooks/useSystemBridge.ts` (BackendType, getBackendType, getRelevantVersion)

### 2. Two independent discovery loops on ChromeOS

`ChromeOSBootstrap` (`chromeos-bootstrap.ts`) hardcodes `100.115.92.2` and only knows about Android. `DaemonBridge.connectChromeos()` probes both hosts. On a Crostini-only device:
- Bootstrap probes Android forever (harmless but wasteful — HTTP requests to unreachable IP every 2s)
- DaemonBridge independently finds and connects to Crostini
- Bootstrap state stays `'probing'` while bridge is `'connected'`
- UI has to check both `state.phase === 'connected' || isCrostini` to render correctly

**Files:** `extension/src/lib/chromeos-bootstrap.ts`, `extension/src/lib/daemon-bridge.ts`, `extension/src/sw.ts`

### 3. Backend type inferred from hostname, not reported

The daemon doesn't tell the extension what mode it's running in. The extension guesses via `if (host === 'penguin.linux.test')`. If someone runs the daemon on a different host, none of the Crostini-specific UI kicks in.

**Files:** `desktop/io-daemon/src/standalone.rs` (StatusResponse), `extension/src/lib/daemon-bridge.ts` (CHROMEOS_CROSTINI_HOST checks)

### 4. Storage keys are semantically wrong

`android:authToken`, `android:daemonPort`, `android:daemonHost` are used for Crostini connections too. If a user switches between Android and Crostini, stale cached values could cause the wrong host to be tried first.

**Files:** `extension/src/lib/daemon-bridge.ts` (STORAGE_KEY_*), `extension/src/lib/chromeos-bootstrap.ts` (STORAGE_KEY_*)

### 5. Error messages assume Android

`daemon-bridge.ts:822` throws `'Companion daemon not reachable'` — wrong when Crostini daemon isn't running. `triggerLaunch()` sends an Android intent that does nothing on Crostini.

### 6. The `stop()` bug is a design smell

The reverted fix (stopping bootstrap on Crostini connect) failed because `ChromeOSBootstrap` has no concept of "someone else handled the connection." Its state machine only has `idle | probing | pairing | connecting | connected` — no `'superseded'` state. And `stop()` sets `phase: 'idle'`, which `handleUIPortConnect()` treats as "needs starting."

### 7. UI branching via scattered `isCrostini` checks

`SystemBridgePanelChromeos.tsx` has 4 extra props and multiple `isCrostini` conditionals to hide Play Store links, folder buttons, etc. This grows with each new connection mode.

## Planned Changes (in priority order)

### Phase 1: Small fixes, big impact (do first)

#### 1a. Add `mode` to daemon `/status` response

Add a `mode` field to `StatusResponse` in the Rust standalone module:

```rust
// standalone.rs
pub mode: Option<String>,  // "standalone"
```

The native host path doesn't use `/status`, but if it ever does, it would report `"native-host"`. The Android companion can add `"companion"` later.

This is the single most impactful change — the extension determines what it's talking to without hostname heuristics.

**Files to change:**
- `desktop/io-daemon/src/standalone.rs` — add `mode` field to `StatusResponse`, set to `"standalone"`
- `extension/src/lib/daemon-bridge/chromeos/http-api.ts` — add `mode` to `ChromeosDaemonStatus`
- `extension/src/lib/daemon-bridge.ts` — use `status.mode` instead of `host === CHROMEOS_CROSTINI_HOST`
- `packages/client/src/hooks/useSystemBridge.ts` — use mode from `daemonInfo` instead of hostname

#### 1b. Add `superseded` phase to ChromeOSBootstrap

Add a state that means "another connection path succeeded, stop probing":

```typescript
export type BootstrapPhase =
  | 'idle' | 'probing' | 'pairing' | 'connecting' | 'connected'
  | 'superseded'  // Another path (e.g. Crostini) connected

// In ChromeOSBootstrap:
supersede(): void {
  this.running = false
  if (this.pollTimer) { clearTimeout(this.pollTimer); this.pollTimer = null }
  this.updateState({ phase: 'superseded', problem: null, message: 'Connected via other path' })
}
```

In `sw.ts`, when DaemonBridge connects and mode is standalone/Crostini:
```typescript
bridge.subscribe((state) => {
  if (state.status === 'connected' && state.daemonInfo?.mode === 'standalone' && chromeosBootstrap) {
    chromeosBootstrap.supersede()
  }
})
```

In `handleUIPortConnect()`:
```typescript
if (state.phase === 'idle') {  // 'superseded' won't match — no restart
  chromeosBootstrap.start()
}
```

**Files to change:**
- `extension/src/lib/chromeos-bootstrap.ts` — add `'superseded'` phase, add `supersede()` method
- `extension/src/sw.ts` — call `supersede()` when Crostini bridge connects; don't restart superseded bootstrap

### Phase 2: Type system cleanup

#### 2a. Replace `BackendType` with `ConnectionMode`

```typescript
type ConnectionMode =
  | 'native-host'       // Desktop: extension + native messaging + io-daemon
  | 'tauri'             // Tauri standalone: webview hosts UI directly
  | 'android-companion' // ChromeOS: extension + Android app
  | 'crostini-daemon'   // ChromeOS: extension + standalone io-daemon in Linux VM
```

Derive from `daemonInfo.mode` (from `/status`) rather than hostname or platform. Version requirements become:

```typescript
const VERSION_REQUIREMENTS: Partial<Record<ConnectionMode, VersionReq>> = {
  'native-host': { minSupported: '0.1.28', recommended: '0.1.28' },
  'android-companion': { minSupported: '1.0.22', recommended: '1.0.22' },
  // 'tauri': no check (self-hosted)
  // 'crostini-daemon': same io-daemon version range as native-host
}
```

**Files to change:**
- `packages/client/src/hooks/useSystemBridge.ts` — replace `BackendType` with `ConnectionMode`, update `getBackendType`, `getRelevantVersion`, `getVersionStatus`
- `packages/client/src/components/SystemBridgePanel.tsx` — update exported type
- `packages/client/src/components/SystemBridgePanelChromeos.tsx` — use `connectionMode` instead of `backendType` + `isCrostini`
- `packages/client/src/App.tsx` — update prop passing

#### 2b. Rename storage keys to be backend-agnostic

```
android:authToken  →  daemon:authToken
android:daemonPort →  daemon:port
android:daemonHost →  daemon:host
```

Add one-time migration on extension update to rename existing keys (in `sw.ts` `onInstalled` handler).

**Files to change:**
- `extension/src/lib/daemon-bridge.ts` — rename constants
- `extension/src/lib/chromeos-bootstrap.ts` — rename constants
- `extension/src/sw.ts` — add migration in `onInstalled`

### Phase 3: Architecture cleanup (when doing significant ChromeOS work)

#### 3a. Fold `ChromeOSBootstrap` into `DaemonBridge`

`ChromeOSBootstrap` was built when Android was the only ChromeOS path. Now there are two. Rather than teaching Bootstrap about Crostini, merge its logic into DaemonBridge:

- `DaemonBridge.connectChromeos()` already does discovery across both hosts
- Move Bootstrap's state progression (probing → pairing → connecting → connected) into DaemonBridge state
- Add `connectionPhase` field to `DaemonBridgeState` so UI can show progress
- Remove `ChromeOSBootstrap` as a separate class
- Service worker no longer coordinates two independent systems

**Files to change:**
- `extension/src/lib/daemon-bridge.ts` — absorb bootstrap logic, add `connectionPhase` to state
- `extension/src/lib/chromeos-bootstrap.ts` — delete
- `extension/src/sw.ts` — remove bootstrap instantiation and coordination code
- `packages/client/src/components/SystemBridgePanelChromeos.tsx` — read from unified bridge state

#### 3b. Separate `ConnectedContent` by connection mode

Instead of `isCrostini` prop spreading through the component, render mode-specific content:

```tsx
{connectionMode === 'android-companion' && <AndroidConnectedContent ... />}
{connectionMode === 'crostini-daemon' && <CrostiniConnectedContent ... />}
```

Each component only renders what's relevant — no conditional hiding. Makes it natural to add future modes.

**Files to change:**
- `packages/client/src/components/SystemBridgePanelChromeos.tsx` — split `ConnectedContent` into mode-specific components

## Implementation Notes

- Phase 1 changes are backward-compatible — old daemons that don't report `mode` still work (fall back to hostname detection).
- Phase 2 can be done independently of Phase 1 but benefits from having `mode` available.
- Phase 3 is a larger refactor best done when you're already doing significant ChromeOS work.
- All phases maintain the existing protocol — no daemon/extension version coupling required.
