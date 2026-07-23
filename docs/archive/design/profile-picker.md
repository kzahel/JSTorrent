# Profile Picker & Rename — Implementation Plan

## Overview

Add `listProfiles` and `renameProfile` operations to the native host, expose them through all client channels, and add a "Profiles" section to Settings UI.

## 1. Native Host — New Operations (Rust)

### `desktop/host/src/protocol.rs`

Add two new operations and one new response payload:

```rust
// In Operation enum:
ListProfiles,
RenameProfile {
    #[serde(rename = "profileId")]
    profile_id: String,
    #[serde(rename = "displayName")]
    display_name: String,
},

// In ResponsePayload enum:
ProfileList {
    profiles: Vec<ProfileListEntry>,
},
```

New struct (in protocol.rs or a shared location):

```rust
#[derive(Debug, Serialize)]
pub struct ProfileListEntry {
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub created: u64,
    #[serde(rename = "lastUsed")]
    pub last_used: u64,
    #[serde(rename = "clientType")]
    pub client_type: Option<String>,
    #[serde(rename = "clientVersion")]
    pub client_version: Option<String>,
    pub live: bool,
}
```

Add `Display` impls for the new variants.

### `desktop/host/src/main.rs`

Add match arms in `handle_request`:

**ListProfiles**: Read `rpc::read_discovery_file()`, for each profile entry check liveness via `rpc::check_profile_liveness(port, &token)`. Fire all health checks concurrently (`futures::join_all`) so total latency is ~100ms regardless of profile count. Return `ProfileList`. No handshake required — works pre-handshake and post-handshake.

**RenameProfile**: Call `rpc::rename_profile(profile_id, display_name)` — a targeted read-modify-write of rpc-info.json that only updates the `display_name` field for the matching profile. No handshake required. Also update in-memory state if the renamed profile is our current profile.

### `desktop/host/src/rpc.rs`

**Reduce liveness timeout**: Change `check_profile_liveness` timeout from 2 seconds to 100ms. This is localhost — if `/health` doesn't respond in 100ms, the daemon is dead.

```rust
pub async fn check_profile_liveness(port: u16, token: &str) -> bool {
    let url = format!("http://127.0.0.1:{port}/health?token={token}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build();
    let Ok(client) = client else { return false };
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
```

**Add `rename_profile` helper**: Targeted read-modify-write that only touches `display_name` for one profile entry. Do NOT reuse `write_discovery_file()` (which takes `RpcWriteInfo` with daemon fields). Instead:

```rust
pub fn rename_profile(profile_id: &str, display_name: &str) -> Result<(), anyhow::Error> {
    let mut rpc_info = read_discovery_file();
    let entry = rpc_info.profiles.iter_mut()
        .find(|p| p.profile_id == profile_id)
        .ok_or_else(|| anyhow::anyhow!("Profile not found: {}", profile_id))?;
    entry.display_name = display_name.to_string();
    // Atomic write via tempfile + rename (same pattern as write_discovery_file)
    write_rpc_info_atomic(&rpc_info)
}
```

## 2. Extension Service Worker — New Message Types

### `extension/src/sw.ts`

Add handlers in `handleMessage()`:

**LIST_PROFILES**: Call `bridge.listProfiles()` → `sendResponse({ok, profiles})`

**RENAME_PROFILE**: Call `bridge.renameProfile(profileId, displayName)` → `sendResponse({ok})`

**SWITCH_PROFILE**: Profile switch with full session restart:
1. Write `profileId` to `chrome.storage.local` (set if non-null, remove if null)
2. Clear pending events: `chrome.storage.session.remove('pending:nativeEvent')` — prevents stale events from the old profile leaking into the new session
3. `bridge.disconnect()` — kills the native host process (closing the native port kills the child process)
4. `await bridge.connect()` — spawns a new native host, handshakes with new profileId read from storage
5. `sendResponse({ok: true})` — send `ok: true` regardless of whether the new profile is in use (ProfileInUse state will be picked up by the UI after reload via `GET_BRIDGE_STATE`)
6. If `bridge.connect()` throws, `sendResponse({ok: false, error: String(e)})` — UI should show an error instead of blindly reloading

Note: The UI will briefly see `BRIDGE_STATE_CHANGED` messages (disconnected → connecting → connected) via the port before the reload happens. This may trigger `engineManager.reset()` momentarily. This is harmless since the page is about to reload, but worth a code comment.

### `extension/src/lib/daemon-bridge.ts`

Add two new methods on `DaemonBridge`:

```typescript
async listProfiles(): Promise<ProfileListEntry[]>
async renameProfile(profileId: string, displayName: string): Promise<boolean>
```

Both use `sendNativeRequest()` pattern (desktop) — but `listProfiles` needs to return the full payload, not just `{ok}`. Use `sendNativeKvRequest()` pattern instead (which returns full response object).

For ChromeOS: profiles are a desktop/Tauri concept. Return empty list / no-op. The UI section will be hidden on ChromeOS.

## 3. Client HostChannel Interface

### `packages/client/src/host/host-channel.ts`

Add to interface:

```typescript
listProfiles(): Promise<ProfileListEntry[]>
renameProfile(profileId: string, displayName: string): Promise<boolean>
switchProfile(profileId: string | null): Promise<void>
```

### `packages/client/src/host/types.ts`

Add:

```typescript
export interface ProfileListEntry {
  profileId: string
  displayName: string
  created: number
  lastUsed: number
  clientType?: string
  clientVersion?: string
  live: boolean
}
```

### `packages/client/src/host/chrome-extension-channel.ts`

Implement via `sendMessage`:
- `listProfiles()`: `{type: 'LIST_PROFILES'}` → parse response `.profiles`
- `renameProfile()`: `{type: 'RENAME_PROFILE', profileId, displayName}` → return `response.ok`
- `switchProfile()`: Send `{type: 'SWITCH_PROFILE', profileId}` → check response → if `response.ok`, call `window.location.reload()`. If `!response.ok`, throw error (don't reload).

### `packages/client/src/host/tauri-channel.ts`

Implement via `hostMessage`:
- `listProfiles()`: `{op: 'listProfiles'}` → parse response
- `renameProfile()`: `{op: 'renameProfile', profileId, displayName}` → return `response.ok`
- `switchProfile()`: write to localStorage → `tauriInvoke('restart_app')`

## 4. Settings UI — Profiles Tab

### `packages/client/src/components/SettingsOverlay.tsx`

**New 5th tab**: "Profiles" in the sidebar. Only visible when `platform === 'desktop' || platform === 'tauri'` (not ChromeOS — profiles are a desktop concept).

Add to `TABS` array and `SettingsTab` type. The tab renders a `ProfilesTab` component.

**ProfilesTab component** (inline in SettingsOverlay.tsx):

- Fetch profiles on tab activation (not just mount) — re-fetch whenever the Profiles tab becomes the active tab, so liveness and metadata are fresh:
  ```typescript
  useEffect(() => {
    if (activeTab === 'profiles') {
      loadProfiles()
    }
  }, [activeTab])
  ```
- Poll `listProfiles()` every 5 seconds while the tab is active, to keep liveness indicators up to date. Cancel the interval when the tab is deactivated or settings close.
- Show loading spinner on first fetch only (subsequent polls update silently)
- Display each profile as a row in a Section:
  - Display name (text, or input field when editing)
  - Pencil icon button to enter inline edit mode
  - Sublabel: "Last used {relative time}" + client type if available
  - Status indicators: "Current" badge (blue) for our profile, "Active" dot (green) if live
  - "Switch" button for non-current profiles
- "+ Create New Profile" button at bottom

Current profile ID = `channel.getState().daemonInfo?.profileId`

**Rename flow**:
1. Click pencil → display name becomes a text input (pre-filled)
2. Press Enter or blur → call `channel.renameProfile(profileId, newName)`
3. On success, update local profiles state
4. On failure, revert to old name

**Switch flow**:
1. Click "Switch" on a different profile
2. Call `channel.switchProfile(profileId)` — stores profileId, reconnects host (extension: SW disconnects+reconnects bridge; Tauri: app restarts)
3. If the call throws, show error in the UI
4. If the target profile is live, the normal ProfileInUse UI will appear with Take Over button after reload/restart

**Create New flow**:
1. Click "Create New Profile"
2. Call `channel.switchProfile(null)` — clears stored profileId, reconnects host
3. Handshake with `profileId: null` → new profile auto-created

## 5. HostChannel — switchProfile method

The DaemonBridge and native messaging port live in the **service worker** (extension) or **Rust backend** (Tauri), NOT the UI page. A simple `window.location.reload()` alone won't restart the native host connection. Each platform needs server-side action to reconnect with the new profile.

### `packages/client/src/host/host-channel.ts`

```typescript
/** Store a new profile ID (or null to create new) and restart the host connection. */
switchProfile(profileId: string | null): Promise<void>
```

### chrome-extension-channel.ts

Send `{type: 'SWITCH_PROFILE', profileId}` to SW → check response → reload only on success.

```typescript
async switchProfile(profileId: string | null): Promise<void> {
  const response = await sendMessage({type: 'SWITCH_PROFILE', profileId})
  if (!response.ok) {
    throw new Error(response.error ?? 'Profile switch failed')
  }
  window.location.reload()
}
```

**SW handler** (`extension/src/sw.ts`):
1. Write `profileId` to `chrome.storage.local` (set if non-null, remove if null)
2. Clear `chrome.storage.session` pending events (prevent old-profile events leaking)
3. `bridge.disconnect()` — kills the native host process
4. `await bridge.connect()` — spawns a new native host, handshakes with new profileId
5. `sendResponse({ok: true})` — regardless of ProfileInUse (UI handles that state after reload)
6. On error: `sendResponse({ok: false, error: ...})`

The UI page reload after a successful response ensures engine + config state reset cleanly.

### tauri-channel.ts

1. Write `profileId` to localStorage (setItem or removeItem)
2. Call `tauriInvoke('restart_app')` — Tauri v2's `AppHandle::restart()` relaunches the entire app

**Tauri backend** (`desktop/tauri-app/src-tauri/src/lib.rs`):
Add a `restart_app` command:
```rust
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}
```
This kills the sidecar (native host), relaunches the app, which re-handshakes with the new profileId from localStorage on startup.

## Files to modify

| File | Changes |
|------|---------|
| `desktop/host/src/protocol.rs` | Add `ListProfiles`, `RenameProfile` operations, `ProfileList` payload, `ProfileListEntry` struct |
| `desktop/host/src/main.rs` | Add match arms for new operations, concurrent liveness checks |
| `desktop/host/src/rpc.rs` | Add `rename_profile()` helper, reduce `check_profile_liveness` timeout to 100ms |
| `extension/src/lib/daemon-bridge.ts` | Add `listProfiles()`, `renameProfile()` methods |
| `extension/src/sw.ts` | Add `LIST_PROFILES`, `RENAME_PROFILE`, `SWITCH_PROFILE` handlers |
| `packages/client/src/host/types.ts` | Add `ProfileListEntry` type |
| `packages/client/src/host/host-channel.ts` | Add `listProfiles()`, `renameProfile()`, `switchProfile()` |
| `packages/client/src/host/chrome-extension-channel.ts` | Implement new methods (switchProfile checks response before reload) |
| `packages/client/src/host/tauri-channel.ts` | Implement new methods + `restart_app` invoke |
| `desktop/tauri-app/src-tauri/src/lib.rs` | Add `restart_app` command |
| `packages/client/src/components/SettingsOverlay.tsx` | Add Profiles tab with polling |

## Verification

1. **Rust**: `cargo clippy --workspace -- -D warnings && cargo test --workspace` in `desktop/`
2. **TypeScript**: `pnpm run typecheck && pnpm run test && pnpm run lint && pnpm format:fix`
3. **Manual test (desktop extension)**: Open extension → Settings → Profiles tab should show current profile with "Current" badge and green "Active" dot. Click pencil to rename. Create new profile (page reloads, new profile active). Switch back to original (page reloads). Verify liveness dots update when a profile's daemon starts/stops.
4. **Manual test (Tauri)**: Same flows as extension. Verify app fully restarts on profile switch (not just page reload).
5. **E2E**: The existing `profile_scenarios.rs` tests validate the core operations. Add test coverage for:
   - `listProfiles` returns correct entries with liveness status
   - `listProfiles` with empty discovery file
   - `renameProfile` updates name and persists across re-read
   - `renameProfile` with nonexistent profile ID returns error
   - Liveness check with 100ms timeout (dead port returns false quickly)
