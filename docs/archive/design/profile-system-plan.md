# Profile System Implementation Plan

## Summary

Replace `installId`-based identity with host-owned **profile IDs**. Minimal rpc-info.json changes: rename fields, add metadata. Profile-scoped PID locking replaces launcher-based mutual exclusion. Per-profile `data.db`. No backwards compat needed.

---

## Phase 1: Rust (host, common, io-daemon)

### 1A. `desktop/common/src/lib.rs` — Struct changes

Rename `UnifiedRpcInfo` → `RpcInfo`. On `ProfileEntry`:

- `install_id: Option<String>` → `profile_id: String` (required, host-generated UUID)
- Add `display_name: String` (default `"Profile 1"`)
- Add `created: u64` (epoch seconds, set once)
- Add `client_type: Option<String>` (`"extension"` | `"tauri"`)
- Add `client_version: Option<String>` (manifest version or cargo version)
- Keep `extension_id`, `pid`, `port`, `token`, `started`, `last_used`, `browser`, `download_roots`, `launcher` as-is

All new fields use `#[serde(default)]` for forward compat.

### 1B. `desktop/host/src/protocol.rs` — Wire protocol

**`Operation::Handshake`**: Accept both old and new fields during transition:
```rust
Handshake {
    extension_id: String,
    #[serde(default)] install_id: Option<String>,    // legacy, ignored after Phase 2
    #[serde(default)] profile_id: Option<String>,    // new: which profile to attach to
    #[serde(default)] client_type: Option<String>,
    #[serde(default)] client_version: Option<String>,
}
```

Same for `Operation::TakeOver`.

**`ResponsePayload::DaemonInfo`**: Add `profile_id: String`.

**Replace `DesktopAppRunning`** with `ProfileInUse`:
```rust
ProfileInUse {
    profile_id: String,
    client_type: Option<String>,
    client_version: Option<String>,
    browser_name: Option<String>,
    pid: u32,
    started: u64,
}
```

Update `Display` impls.

### 1C. `desktop/host/src/state.rs` — Deferred KV, profile tracking

```rust
pub struct State {
    pub event_sender: Option<mpsc::Sender<Event>>,
    pub rpc_info: Mutex<Option<crate::rpc::RpcWriteInfo>>,
    pub kv: Mutex<Option<KvStore>>,        // was KvStore — deferred until handshake
    pub launcher: String,
    pub profile_id: Mutex<Option<String>>, // new: set after handshake
    // blocked_by_tauri: REMOVED
}
```

### 1D. `desktop/host/src/rpc.rs` — Discovery file + liveness check

**Rename local `RpcInfo`** → `RpcWriteInfo` (avoids collision with common `RpcInfo`).
- Replace `install_id: Option<String>` with `profile_id: String`
- Add `client_type`, `client_version`, `display_name` fields

**`write_discovery_file`**: Match by `profile_id` (not install_id, not PID). Simpler logic:
1. Find entry where `entry.profile_id == info.profile_id`
2. If found → update fields (pid, port, token, last_used, client metadata, browser, optionally roots)
3. If not found → create new entry
4. Atomic write (existing temp-file-rename pattern)

Remove the PID-based fallback matching and install_id cleanup code.

**`read_discovery_file`** → returns `jstorrent_common::RpcInfo` (renamed type).

**Add `check_profile_liveness`** (async fn):
```rust
pub async fn check_profile_liveness(port: u16, token: &str) -> bool
```
HTTP GET to `http://127.0.0.1:{port}/health?token={token}` with 2s timeout. Returns true if 200 OK. Uses `reqwest` (already a dependency).

**Update tests**: Adapt `make_rpc_info` helper to use `profile_id: &str` (required). Update all 4 existing tests. Add new tests:
- `test_profile_match_by_id`: write with profile_id X, rewrite with profile_id X → same entry updated
- `test_profile_create_new`: write with profile_id Y when only X exists → new entry added
- `test_profile_metadata_preserved`: verify display_name, created survive updates

### 1E. `desktop/host/src/main.rs` — Core changes

**Remove startup KV init** (lines 57-64). KV opened after handshake.

**Replace incumbent detection** (lines 71-107). Remove the launcher-based matrix and `blocked_by_tauri`. Profile locking now happens at handshake time, not startup.

On startup:
- Write a placeholder entry with `profile_id = format!("pending-{}", std::process::id())` so the RPC server port/token are discoverable for link handling. This entry is replaced on handshake.

**`State::new`** — no `kv` arg, no `blocked_by_tauri` arg.

**`do_handshake` rewrite**:
```rust
async fn do_handshake(
    state: &State,
    extension_id: String,
    profile_id: Option<String>,     // null = auto-resolve or create
    client_type: Option<String>,
    client_version: Option<String>,
    daemon_manager: &mut DaemonManager,
    system: &mut sysinfo::System,
) -> Result<ResponsePayload>
```

Logic:
1. Read rpc-info.json fresh from disk
2. Resolve profile:
   - If `profile_id` is `Some` → find by ID. If not found → error (invalid profile ID).
   - If `profile_id` is `None` → find by `extension_id` match (auto-attach to existing). If none found → create new profile (generate UUID, `display_name = "Profile N"`, `created = now()`).
3. Check incumbent liveness (if profile's PID != our PID and PID != 0):
   - `check_profile_liveness(profile.port, &profile.token).await`
   - If alive → return `ProfileInUse { ... }` with error `"profile_in_use"`
   - If dead → proceed (take over stale profile)
4. Update profile entry: our PID, port, token, last_used, client_type, client_version, browser info. Set `download_roots = None` to preserve existing roots.
5. Write rpc-info.json
6. Open per-profile KV: `profiles/{profile_id}/data.db` (create dir if needed). Store in `state.kv`.
7. Store profile_id in `state.profile_id`.
8. Start daemon with `profile_id`.
9. Return `DaemonInfo { ..., profile_id }`.

**`Operation::Handshake` handler**: Remove `blocked_by_tauri` check. Pass new fields to `do_handshake`. The update-check-after-handshake logic stays but guards on `state.kv` being `Some`.

**`Operation::TakeOver` handler**: Remove launcher-specific kill logic. New flow:
1. Read profile from rpc-info.json (by profile_id or extension_id)
2. If profile's PID is alive → kill it via `sysinfo::Process::kill()`
3. Sleep 500ms (let daemon die via parent-pid monitoring)
4. Proceed with normal `do_handshake` flow

**KV operation handlers** (lines 636-666): Guard with:
```rust
let kv = state.kv.lock().unwrap();
let kv = kv.as_ref().ok_or_else(|| anyhow::anyhow!("Handshake required before KV operations"))?;
```

### 1F. `desktop/host/src/daemon_manager.rs`

- `start(&mut self, install_id: &str)` → `start(&mut self, profile_id: &str)`
- `--install-id` → `--profile-id` in command args (line 42-43)

### 1G. `desktop/io-daemon/src/main.rs`

- `--install-id` CLI arg → `--profile-id` (line 50-51)
- `AppState.install_id` → `AppState.profile_id` (line 112)
- Update `run_managed()` error message (line 194-196)
- Standalone mode: `profile_id` defaults to `"standalone"` (line 264)

### 1H. `desktop/io-daemon/src/config.rs`

- `load_config(install_id: &str)` → `load_config(profile_id: &str)` (line 18)
- Match `p.profile_id == profile_id` (line 35-38)
- `refresh_handler`: `state.install_id` → `state.profile_id` (line 60)

### 1I. `desktop/host/tests/native_messaging.rs`

Update handshake JSON (still send `installId` for Phase 1 legacy compat — host ignores it and creates a profile). Assert `profileId` in response:
```rust
let profile_id = payload["profileId"].as_str().expect("profileId must be present");
assert!(!profile_id.is_empty());
```

### Phase 1 verification

```bash
cd desktop
source ~/.profile
TRIPLE="$(rustc --print host-tuple)"
mkdir -p tauri-app/src-tauri/binaries
touch "tauri-app/src-tauri/binaries/jstorrent-host-$TRIPLE"
touch "tauri-app/src-tauri/binaries/jstorrent-io-daemon-$TRIPLE"
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

---

## Phase 2: Extension + Client (TypeScript)

### 2A. `extension/src/lib/daemon-bridge.ts`

**`connectDesktop()`** (~line 667):
- Remove `const telemetryId = await getOrCreateTelemetryId()` from handshake path
- New handshake message:
  ```typescript
  const stored = await chrome.storage.local.get('profileId')
  const handshakeMsg = {
    op: 'handshake',
    extensionId: chrome.runtime.id,
    profileId: stored.profileId ?? null,
    clientType: 'extension',
    clientVersion: chrome.runtime.getManifest().version,
    id: crypto.randomUUID(),
  }
  ```
- On DaemonInfo response, store profileId:
  ```typescript
  if (payload.profileId) {
    chrome.storage.local.set({ profileId: payload.profileId })
  }
  ```

**Replace `isDesktopAppRunningMessage`** (line 1348) with `isProfileInUseMessage`:
- Check for `error === 'profile_in_use'`
- Extract payload metadata (clientType, clientVersion, browserName, pid, started)
- Reject with descriptive error (not just `'desktop_app_running'`)

**`takeOver()`** (~line 1364):
- Remove telemetryId, send `clientType: 'extension'`, `clientVersion`, `profileId`

**`connectWebSocket()`** (~line 893): Desktop path sends `profileId` (from `chrome.storage.local`) instead of `telemetryId` in AUTH frame.

**`buildHeaders()`** (line 371): No change needed — headers are only for ChromeOS HTTP path which still uses telemetryId.

**ChromeOS paths unchanged**: `checkStatusAndPair`, `pollForPairing`, `connectChromeos`, `fetchStatus`, `requestPairing`, `chromeos-bootstrap.ts` all continue using `telemetryId` for Android companion pairing. Separate system.

### 2B. `extension/src/sw.ts`

- `isExtensionLocalKey` (line 390): Add `'profileId'` alongside `'telemetryId'`
- `onInstalled` handler (line 281-286): Remove `telemetryId` call (keep for metrics elsewhere, not needed on install for handshake)
- `TAKE_OVER_FROM_DESKTOP` handler: Still works, no change needed

### 2C. `packages/client/src/components/SystemBridgePanel.tsx`

- Replace `desktop_app_running` string checks (line 236, 552) with `profile_in_use`
- Update "Desktop App Running" message to show metadata: "This profile is in use by {clientType} v{clientVersion}"
- Rename `onTakeOverFromDesktop` prop to `onTakeOver` (it's no longer desktop-specific)

### 2D. `packages/client/src/App.tsx`

- Update `desktop_app_running` check (line 436) to `profile_in_use`
- Update overlay text to show who's using the profile
- Rename prop

### 2E. `packages/client/src/hooks/useIOBridgeState.ts`

- Rename `takeOverFromDesktop` (line 30, 102, 116) to `takeOver`

### 2F. `packages/client/src/host/chrome-extension-channel.ts`

- Rename `takeOverFromDesktop` (line 323) to `takeOver`
- Message type `TAKE_OVER_FROM_DESKTOP` → `TAKE_OVER` (or keep for compat, just rename the method)

### 2G. `packages/client/src/engine-manager/daemon-engine-manager.ts`

- `createCredentialsGetter` (line 62-79): For ChromeOS path, keep reading `telemetryId`. For desktop path, read `profileId` from storage. The `installId` field in the credentials maps to `profileId` for the io-daemon's AUTH frame.

### 2H. `extension/src/lib/telemetry-id.ts` — No changes

Stays for metrics + ChromeOS companion pairing.

### Phase 2 verification

```bash
pnpm run typecheck
pnpm run test
pnpm run lint
pnpm format:fix
```

Manual: Load extension, connect to host from Phase 1, verify profileId stored in chrome.storage.local. Open second Chrome profile, verify profile_in_use error with metadata.

---

## Phase 3: Tauri App

### 3A. `desktop/tauri-app/src-tauri/src/lib.rs`

- **Delete `get_or_create_install_id()`** (lines 272-288) and the `install-id` file handling
- **`host_handshake()`** (lines 332-345):
  ```rust
  let response = state.request(json!({
      "op": "handshake",
      "extensionId": "tauri-desktop",
      "clientType": "tauri",
      "clientVersion": env!("CARGO_PKG_VERSION"),
  })).await?;
  ```
  No profileId sent on first connect (host creates one). On subsequent connects, could store and resend, but simplest: always let host auto-resolve by extensionId.

### 3B. Tauri frontend JS

- Handle `profile_in_use` error from handshake response (currently handles `desktop_app_running`)
- Show "Profile in use by {clientType} v{clientVersion}" with Take Over button
- Store `profileId` from successful handshake in localStorage for display purposes

### Phase 3 verification

```bash
cd desktop
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Manual: Launch Tauri, verify handshake gets profileId. With extension connected, launch Tauri, verify profile_in_use. TakeOver from either side works.

---

## Files Changed Summary

### Phase 1 (Rust)
| File | Change |
|------|--------|
| `desktop/common/src/lib.rs` | Rename `UnifiedRpcInfo` → `RpcInfo`, `install_id` → `profile_id`, add metadata fields |
| `desktop/host/src/protocol.rs` | Add profile_id to Handshake/TakeOver/DaemonInfo, add ProfileInUse, remove DesktopAppRunning |
| `desktop/host/src/state.rs` | `kv: Option<KvStore>`, add `profile_id`, remove `blocked_by_tauri` |
| `desktop/host/src/rpc.rs` | Rename local struct, match by profile_id, add `check_profile_liveness`, update tests |
| `desktop/host/src/main.rs` | Remove startup KV + incumbent matrix, rewrite do_handshake with profile resolution + liveness, guard KV ops |
| `desktop/host/src/daemon_manager.rs` | `install_id` → `profile_id`, `--install-id` → `--profile-id` |
| `desktop/io-daemon/src/main.rs` | CLI arg + AppState rename |
| `desktop/io-daemon/src/config.rs` | Match by `profile_id` |
| `desktop/host/tests/native_messaging.rs` | Assert profileId in response |

### Phase 2 (Extension + Client)
| File | Change |
|------|--------|
| `extension/src/lib/daemon-bridge.ts` | Send profileId + client metadata, receive profileId, handle profile_in_use |
| `extension/src/sw.ts` | Add profileId to isExtensionLocalKey |
| `packages/client/src/components/SystemBridgePanel.tsx` | profile_in_use UI with metadata |
| `packages/client/src/App.tsx` | profile_in_use overlay |
| `packages/client/src/hooks/useIOBridgeState.ts` | Rename takeOver |
| `packages/client/src/host/chrome-extension-channel.ts` | Rename takeOver |
| `packages/client/src/engine-manager/daemon-engine-manager.ts` | Read profileId for credentials |

### Phase 3 (Tauri)
| File | Change |
|------|--------|
| `desktop/tauri-app/src-tauri/src/lib.rs` | Delete install_id, send clientType/clientVersion |
| Tauri frontend JS | Handle profile_in_use, store profileId |
