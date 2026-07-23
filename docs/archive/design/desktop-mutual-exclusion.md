# Desktop Mutual Exclusion: Tauri App vs Chrome Extension

## Problem

The `jstorrent-host` binary is shared between two launchers:

1. **Tauri app** spawns it as a sidecar child process (`std::process::Command` in `desktop/tauri-app/src-tauri/src/lib.rs:530`)
2. **Chrome extension** spawns it via native messaging (`chrome.runtime.connectNative` in `extension/src/lib/daemon-bridge.ts:638`)

Both instances independently:
- Open the same SQLite `data.db` (safe with WAL + busy_timeout since v0.1.16)
- Write to `~/.config/jstorrent-native/rpc-info.json` (multi-profile discovery file)
- Spawn their own `io-daemon` (torrent engine) on handshake

There is no coordination between them. Running both simultaneously means two daemons, racing discovery file writes, and undefined behavior.

## Desired Behavior

Modeled after Android's standalone/companion mutual exclusion (`NativeStandaloneActivity` ↔ `IoDaemonService`):

| Scenario | Behavior |
|----------|----------|
| User opens Tauri app while extension is connected | Tauri silently kills extension's native host. Extension disconnects. |
| Extension connects while Tauri app is running | Extension UI shows "Desktop App Running" with "Quit Desktop & Use Extension" button. |
| User clicks "Quit Desktop & Use Extension" | Extension sends TakeOver → kills Tauri sidecar → Tauri app exits → extension spawns daemon normally. |
| Only one is running | Normal operation, no coordination needed. |

**Key invariant:** At most one `io-daemon` running at any time.

## Detection Mechanism

### Launcher Identity

Add a `launcher` field to `ProfileEntry` in rpc-info.json:

```json
{
  "version": 1,
  "profiles": [
    {
      "launcher": "tauri",
      "pid": 1234,
      "extension_id": "tauri-desktop",
      "install_id": "...",
      ...
    }
  ]
}
```

Values: `"tauri"`, `"chrome"`, or absent (legacy entries treated as `"chrome"`).

The Tauri app passes `--launcher tauri` when spawning its sidecar. The native host defaults to `"chrome"` when no `--launcher` arg is present (Chrome doesn't pass extra args).

### Incumbent Detection

On startup, after `system.refresh_processes()`, the native host:
1. Reads rpc-info.json
2. For each profile, checks if PID is alive via `sysinfo::System::process(pid)`
3. Applies the behavior matrix

### Behavior Matrix

| I am | Incumbent | Action |
|------|-----------|--------|
| `tauri` | Live `chrome` | Kill Chrome's native host PID (its daemon dies via parent-pid monitoring), proceed |
| `tauri` | Stale `tauri` | Clean stale profile, proceed |
| `chrome` | Live `tauri` | Set `blocked_by_tauri = Some(pid)`, return error on handshake |
| `chrome` | Live/stale `chrome` | Proceed normally (old one is stale or being replaced) |

### Daemon Lifecycle

The io-daemon already has `--parent-pid` monitoring (`desktop/io-daemon/src/main.rs`). When a native host is killed, its daemon self-terminates within ~1 second. No extra cleanup needed.

## Implementation

### Phase 1: Rust — Profile Identity

**`desktop/common/src/lib.rs`**

Add to `ProfileEntry`:
```rust
#[serde(default)]
pub launcher: Option<String>,
```

`#[serde(default)]` ensures backward compatibility with existing rpc-info.json files.

### Phase 2: Rust — Protocol

**`desktop/host/src/protocol.rs`**

Add `Operation` variant:
```rust
TakeOver {
    #[serde(rename = "extensionId")]
    extension_id: String,
    #[serde(rename = "installId")]
    install_id: String,
},
```

Add `ResponsePayload` variant:
```rust
DesktopAppRunning {
    tauri_pid: u32,
},
```

### Phase 3: Rust — State

**`desktop/host/src/state.rs`**

Add fields:
```rust
pub launcher: String,                      // "tauri" or "chrome"
pub blocked_by_tauri: Mutex<Option<u32>>,  // Some(pid) if Tauri is running
```

### Phase 4: Rust — Startup Coordination

**`desktop/host/src/main.rs`**

New startup sequence (between KV store open and discovery file write):

```
1. Parse --launcher arg (default: "chrome")
2. system.refresh_processes()
3. Read rpc-info.json, check for live incumbents
4. If tauri-launched and chrome incumbent alive:
     system.process(pid).kill()
5. If chrome-launched and tauri incumbent alive:
     blocked_by_tauri = Some(pid)
6. Clean stale profiles (dead PIDs)
7. Create State with launcher + blocked flag
8. Continue with existing startup (RPC server, browser detection, discovery file write)
```

Include `launcher` in the `RpcInfo` struct passed to `write_discovery_file()`.

**Handshake handler** (existing `Operation::Handshake` match arm):
- At the top, check `state.blocked_by_tauri`
- If `Some(pid)`, return `Response { ok: false, error: "desktop_app_running", payload: DesktopAppRunning { tauri_pid } }`

**New TakeOver handler:**
```
1. Read blocked_by_tauri to get Tauri PID
2. system.refresh_processes(); system.process(pid).kill()
3. Clear blocked_by_tauri to None
4. tokio::time::sleep(500ms) — let daemon die via parent-pid monitoring
5. Proceed with handshake logic (extract into shared helper to avoid duplication):
   update discovery file, start daemon, return DaemonInfo
```

Pass `&mut sysinfo::System` to `handle_request()` (currently a local in `main()`).

### Phase 5: Rust — Discovery File Writer

**`desktop/host/src/rpc.rs`**

Add `pub launcher: Option<String>` to `RpcInfo` struct.

In `write_discovery_file()`:
- Propagate `launcher` to new `ProfileEntry` creation
- Update `launcher` on existing entries when provided

### Phase 6: Tauri App — Sidecar Args + Death Handling

**`desktop/tauri-app/src-tauri/src/lib.rs`**

Pass launcher arg when spawning sidecar (~line 533):
```rust
cmd.arg("--launcher").arg("tauri")
```

Handle sidecar death: after `run_stdout_reader` loop exits (stdout EOF), call `app_handle.exit(0)`. This handles the TakeOver case where the extension kills our sidecar.

### Phase 7: Extension — Handle Blocked State

**`extension/src/lib/daemon-bridge.ts`**

In `connectDesktop()` message handler, before the `isDaemonInfoMessage` check:
- Detect `{ ok: false, error: "desktop_app_running" }` response
- Set `resolved = true`, clear timeout
- Save port to `this.nativePort` (keep alive for TakeOver)
- Reject with `new Error('desktop_app_running')`

The `doConnect()` catch handler sets `lastError: 'desktop_app_running'`. No new state field needed — the UI checks this specific `lastError` string.

New `takeOver()` method:
- Sends `{ op: "takeOver", extensionId, installId, id }` via saved `this.nativePort`
- Waits for `DaemonInfo` response (15s timeout — needs time for Tauri to die + daemon to start)
- On success: updates state to connected, starts health check

### Phase 8: Extension Service Worker

**`extension/src/sw.ts`**

Add handler for `TAKE_OVER_FROM_DESKTOP` message type:
```typescript
if (message.type === 'TAKE_OVER_FROM_DESKTOP') {
  bridge.takeOver().then(ok => sendResponse({ ok }))
  return true // async
}
```

### Phase 9: Extension UI

**`packages/client/src/components/SystemBridgePanel.tsx`**

Add `onTakeOverFromDesktop?: () => void` prop.

In `renderContent()` under `disconnected` → `desktop`:
- When `state.lastError === 'desktop_app_running'`: show "Desktop App Running" with explanation

In `renderActions()`:
- When `lastError === 'desktop_app_running'` and `onTakeOverFromDesktop` is set: show "Quit Desktop App & Use Extension" button

**`packages/client/src/host/chrome-extension-channel.ts`**

Add `takeOverFromDesktop()` method that sends `TAKE_OVER_FROM_DESKTOP` to the SW.

Wire the prop through the extension's App.tsx to SystemBridgePanel.

## Edge Cases

1. **Race: both start simultaneously.** The incumbent check reads rpc-info.json before writing. If both start at the exact same instant, neither sees the other. Both write profiles and spawn daemons. Mitigation: extremely unlikely in practice (Tauri sidecar starts in setup(), Chrome's native host starts on user action). If it happens, one will see the other on next extension reconnect.

2. **Chrome restarts native host after kill.** Chrome may re-launch the native host if the extension tries to reconnect. The new host will detect the Tauri incumbent and block again. Stable state is reached.

3. **Tauri app launched while extension is taking over.** Tauri will see Chrome's profile is alive and kill it. Extension disconnects. User can try again. This is a narrow timing window.

4. **Stale PID matches a different process.** Unlikely but possible. Mitigation: could verify process name contains "jstorrent" before killing. For v1, accept the risk (PIDs are written recently and checked immediately).

5. **Windows process killing.** `sysinfo::Process::kill()` uses `TerminateProcess` on Windows. Works but is ungraceful. The io-daemon's parent-pid monitoring handles daemon cleanup regardless.

## File Summary

| File | Change |
|------|--------|
| `desktop/common/src/lib.rs` | Add `launcher: Option<String>` to `ProfileEntry` |
| `desktop/host/src/protocol.rs` | Add `TakeOver` op, `DesktopAppRunning` response |
| `desktop/host/src/state.rs` | Add `launcher`, `blocked_by_tauri` fields |
| `desktop/host/src/main.rs` | Startup coordination, handshake blocking, TakeOver handler |
| `desktop/host/src/rpc.rs` | Add `launcher` to `RpcInfo`, propagate in write |
| `desktop/tauri-app/src-tauri/src/lib.rs` | Pass `--launcher tauri`, exit on sidecar death |
| `extension/src/lib/daemon-bridge.ts` | Handle `desktop_app_running`, `takeOver()` method |
| `extension/src/sw.ts` | Forward `TAKE_OVER_FROM_DESKTOP` message |
| `packages/client/src/components/SystemBridgePanel.tsx` | "Desktop App Running" UI state |
| `packages/client/src/host/chrome-extension-channel.ts` | `takeOverFromDesktop()` method |

## Implementation Order

Phases 1-6 are Rust-only. Phases 7-9 are TypeScript-only. They can be done in two PRs.

**Verification:**
```bash
# Rust
cd desktop && cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace

# TypeScript
pnpm run typecheck && pnpm run test && pnpm run lint && pnpm format:fix
```
