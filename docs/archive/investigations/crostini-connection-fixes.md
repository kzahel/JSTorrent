# Crostini Connection Path: Current Fixes and Refactoring Notes

## Background

The Crostini (ChromeOS Linux VM) connection path uses the desktop `io-daemon` in `--standalone` mode, running at `penguin.linux.test:7800`. Unlike the desktop native host path, there's no native messaging — the extension discovers the daemon via HTTP probing and communicates over HTTP + WebSocket.

The problem: the extension's ChromeOS code was built entirely around the Android companion app. The Crostini daemon was grafted on as a special case, requiring fixes scattered across Rust, the client UI, and the service worker. This doc records what was done and what didn't work.

## Changes Made

### 1. Rust: camelCase serialization (committed: 177160df)

**File:** `desktop/io-daemon/src/standalone.rs`

The standalone `/status` endpoint returned snake_case JSON keys (`extension_id`, `install_id`, `token_valid`). The extension expected camelCase (`extensionId`, `installId`, `tokenValid`). This caused pairing to loop forever — `extensionId` was always `undefined`, so the extension thought it wasn't paired, tried to pair again, got 409 Conflict, retried.

Fix: `#[serde(rename_all = "camelCase")]` on `StatusResponse`. Also added `io_port` field — without it, `completeConnection()` threw "ioPort not provided".

### 2. Client: version check backend type (committed: 575a73ea)

**File:** `packages/client/src/hooks/useSystemBridge.ts`

`getBackendType()` returned `'android'` for all ChromeOS platforms. The Crostini daemon reports version `0.1.30` (desktop io-daemon versioning). Android minimum is `1.0.22`. So `0.1.30 < 1.0.22` → "Update Required" block.

Fix: if `daemonInfo.host === 'penguin.linux.test'`, return `'desktop'` instead of `'android'`. Desktop minimum is `0.1.28`, so `0.1.30 >= 0.1.28` passes.

### 3. Rust: ensure download root directory exists (uncommitted)

**File:** `desktop/io-daemon/src/main.rs`

The daemon's `create_download_root()` runs `canonicalize()` + stat on the path. If the directory doesn't exist, `last_stat_ok` is `false`, causing the UI to show a "Setup" warning badge even though the connection is healthy.

Fix: `create_dir_all` before `create_download_root()`.

### 4. Client UI: Crostini mode in System Bridge panel (uncommitted)

**Files:** `packages/client/src/components/SystemBridgePanelChromeos.tsx`, `packages/client/src/App.tsx`

The ChromeOS panel is driven by `chromeosBootstrapState`, which probes `100.115.92.2` (the Android ARC container IP). It never finds a Crostini daemon — bootstrap is stuck in "probing" forever while the daemon bridge is separately connected.

Fix: pass `backendType`, `ioBridgeConnected`, `daemonHost`, `daemonPort` into the panel. When `backendType === 'desktop' && ioBridgeConnected`, treat as connected regardless of bootstrap phase. Show "Crostini Daemon" instead of "Android App", hide "Add Folder" button (roots are set via `--download-root` flag), hide Play Store update links.

### 5. Service worker: stop bootstrap probing (attempted, reverted)

**File:** `extension/src/sw.ts`

Attempted to call `chromeosBootstrap.stop()` when the daemon bridge connected to `penguin.linux.test`. This didn't work because `stop()` sets `phase: 'idle'`, and the next UI port connection checks `if (state.phase === 'idle') { bootstrap.start() }` — immediately restarting the probe loop.

This was reverted. The bootstrap probing continues in the background even when Crostini is connected. It's harmless (just polls `100.115.92.2` which doesn't respond) but wasteful.

## What's Wrong With This Approach

The Crostini path is bolted on as a series of `if (host === 'penguin.linux.test')` checks and `if (isCrostini)` branches. Problems:

1. **Host detection is fragile.** `penguin.linux.test` is the default Crostini hostname but can be changed. If someone runs the daemon on a different host, none of the Crostini-specific UI kicks in.

2. **Two parallel connection systems.** `ChromeOSBootstrap` and `DaemonBridge` both run on ChromeOS. Bootstrap handles Android discovery/pairing. DaemonBridge handles Crostini (and desktop). They don't coordinate — bootstrap keeps probing even when DaemonBridge is connected.

3. **Backend type is inferred from host, not reported.** The daemon doesn't tell the extension "I'm a Crostini standalone daemon." The extension guesses based on hostname. The daemon should report its mode in `/status`.

4. **UI branching via props.** `SystemBridgePanelChromeos` has grown 4 new props and scattered `isCrostini` conditionals. This would be cleaner as a separate panel or a shared panel with a mode enum.

5. **Version requirements are awkward.** The Crostini daemon uses desktop io-daemon versioning (`0.x`) but conceptually it's a ChromeOS backend. The version check had to special-case it to avoid comparing against Android version requirements.

## Recommended Refactoring

Make Crostini a first-class connection type:

1. **Add `mode` to `/status` response.** Something like `"mode": "standalone"` vs `"mode": "native-host"`. The extension can use this instead of hostname detection.

2. **Unify ChromeOS connection into DaemonBridge.** DaemonBridge already probes both `100.115.92.2` (Android) and `penguin.linux.test` (Crostini). ChromeOSBootstrap duplicates the Android probing. Consider making DaemonBridge the sole connection manager on ChromeOS, with bootstrap folded in or removed.

3. **Add a `BackendType` or connection mode to DaemonBridgeState.** Instead of inferring from hostname, have the bridge report what it connected to. Derive UI behavior from this.

4. **Extract panel rendering by mode.** Instead of `isCrostini` checks in `ConnectedContent`, render a different connected content component based on `backendType`.

## Current Diffs

### Committed

#### 177160df — Fix standalone /status response

```diff
--- a/desktop/io-daemon/src/standalone.rs
+++ b/desktop/io-daemon/src/standalone.rs
+    /// WebSocket port for /control endpoint (same as port in standalone mode)
+    pub io_port: Option<u16>,

+        io_port: Some(state.port),

+#[cfg(test)]
+mod tests {
+    use super::*;
+    #[test]
+    fn status_response_uses_camel_case_keys() { ... }
+}
```

Note: `#[serde(rename_all = "camelCase")]` was added to `StatusResponse` but the diff shows it was already in place via a prior edit to the struct definition line — the key addition was `io_port` and the test.

#### 575a73ea — Fix version check backend type

```diff
--- a/packages/client/src/hooks/useSystemBridge.ts
+++ b/packages/client/src/hooks/useSystemBridge.ts
 function getBackendType(state: DaemonBridgeState): BackendType {
   if (state.platform === 'tauri') return 'self'
-  if (state.platform === 'chromeos') return 'android'
+  if (state.platform === 'chromeos') {
+    if (state.daemonInfo?.host === 'penguin.linux.test') return 'desktop'
+    return 'android'
+  }
   return 'desktop'
 }
```

### Uncommitted

#### main.rs — Ensure download root exists

```diff
+    if !download_root_path.exists() {
+        std::fs::create_dir_all(&download_root_path)?;
+        tracing::info!("Created download root directory: {:?}", download_root_path);
+    }
```

#### SystemBridgePanelChromeos.tsx — Crostini UI mode

Added props: `backendType`, `ioBridgeConnected`, `daemonHost`, `daemonPort`.
Added `isCrostini` flag. When true: shows "Crostini Daemon", hides "Add Folder", hides Play Store links, shows actual daemon host instead of `100.115.92.2`.

#### App.tsx — Pass new props

```diff
+                  backendType={systemBridge.backendType}
+                  ioBridgeConnected={ioBridgeState.status === 'connected'}
+                  daemonHost={ioBridgeState.daemonInfo?.host ?? undefined}
+                  daemonPort={ioBridgeState.daemonInfo?.port}
```

### Reverted (did not work)

#### sw.ts — Stop bootstrap on Crostini connect

```typescript
// This was added then removed:
if (state.daemonInfo?.host === 'penguin.linux.test' && chromeosBootstrap) {
  console.log('[SW] Connected via Crostini, stopping ChromeOS bootstrap')
  chromeosBootstrap.stop()
}
```

`stop()` sets `phase: 'idle'`, but `handleUIPortConnect()` restarts bootstrap when `phase === 'idle'`. The stop is immediately undone on the next UI connect.
