# Launch Desktop App from Extension

## Goal
Add a button in the extension's Interface settings to launch the Tauri desktop app. The desktop app takes over the current extension profile seamlessly.

## Flow
1. User clicks "Open Desktop App" in extension settings
2. Extension sends `launchDesktop` to its native host (includes current profile ID)
3. Native host finds the Tauri binary (reuses `updater::find_tauri_app_path()`) and spawns it with `--force-desktop --profile <id>`
4. Tauri app starts, `--force-desktop` bypasses extension routing so it doesn't exit
5. Tauri's `host_handshake` uses the `--profile` ID, gets `profile_in_use` (extension still owns it)
6. Because `--force-desktop` is set, Tauri auto-sends `takeOver` → kills extension's native host → takes over the profile
7. Extension loses connection (native host was killed), shows disconnected state
8. User is now in the desktop app with their torrents

## Changes

### 1. Native host protocol — add `LaunchDesktop`
**`desktop/host/src/protocol.rs`**
- Add `LaunchDesktop` to `Operation` enum
- Add to `Display` impl

### 2. Native host — handle `LaunchDesktop`
**`desktop/host/src/main.rs`**
- Handle `Operation::LaunchDesktop` in `handle_request()`
- Read current profile ID from `state.profile_id`
- Call `updater::launch_desktop_app(profile_id)` (new function)
- Return `ResponsePayload::Empty`

**`desktop/host/src/updater.rs`**
- Make `find_tauri_app_path()` → `pub(crate)`
- Add `pub(crate) fn launch_desktop_app(profile_id: Option<&str>) -> Result<()>`
  - Finds Tauri binary via `find_tauri_app_path()`
  - Spawns with `--force-desktop --profile <id>` args
  - macOS: `open -a <path> --args --force-desktop --profile <id>` (no `-W`)
  - Windows/Linux: direct spawn, detached
  - Fire-and-forget (don't wait for child)

### 3. Tauri app — `--force-desktop` + `--profile` flags
**`desktop/tauri-app/src-tauri/src/lib.rs`**

Add `LaunchArgs` struct stored as managed state:
```rust
struct LaunchArgs {
    force_desktop: bool,
    profile_id: Option<String>,
}
```

In `run()` (line ~785):
- Parse `--force-desktop` and `--profile <id>` from `std::env::args()` (alongside existing `--check-update`/`--auto-update`)
- Store as `app.manage(LaunchArgs { ... })`

In `setup()` (line ~1056):
- If `force_desktop` is true, skip `determine_startup_action` — always proceed to desktop path

In `host_handshake` (line ~535):
- Accept `launch_args: tauri::State<'_, LaunchArgs>` parameter
- If `launch_args.profile_id` is Some, override the frontend-provided `profile_id`
- After sending handshake, if response has `error: "profile_in_use"` and `launch_args.force_desktop`:
  - Auto-send `takeOver` message through the same bridge
  - Return the takeover result instead

Single-instance callback (line ~794):
- `--force-desktop` is not a URL, so `handle_deep_link_routed` returns `NotRecognized`
- Fallback `show_main_window()` already handles this case correctly

### 4. DaemonBridge — `launchDesktop()` method
**`extension/src/lib/daemon-bridge.ts`**
- Add `async launchDesktop(): Promise<boolean>`
- Guard: `if (this.state.platform !== 'desktop') return false`
- Uses `sendNativeRequest('launchDesktop', {})`

### 5. Service worker — message handler
**`extension/src/sw.ts`**
- Add `LAUNCH_DESKTOP` handler (same pattern as `CHECK_FOR_UPDATES` at line ~756)
- Calls `bridge.launchDesktop()`

### 6. HostChannel interface + implementations
**`packages/client/src/host/host-channel.ts`**
- Add `launchDesktop(): Promise<boolean>`

**`packages/client/src/host/chrome-extension-channel.ts`**
- Implement: sends `{ type: 'LAUNCH_DESKTOP' }` message

**`packages/client/src/host/tauri-channel.ts`**
- No-op: `return false`

### 7. Settings UI — "Desktop App" section
**`packages/client/src/components/SettingsOverlay.tsx`**

Add props to `InterfaceTabProps`: `platform: string`, `desktopVersion: string | undefined`, `channel: HostChannel`

In `InterfaceTab`, after the Performance section, add (only when `platform === 'desktop' && !isStandalone`):
```
Section title="Desktop App"
  Text: "You can also use JSTorrent as a standalone desktop app."
  Shows: "v{desktopVersion} installed"
  Button: "Open Desktop App" → channel.launchDesktop()
```

Pass the new props from where `InterfaceTab` is rendered (line ~376). `channel` is already available via `useHostChannel()`, and `desktopVersion` comes from `channel.getState().daemonInfo?.desktopVersion`.

## Files Modified
1. `desktop/host/src/protocol.rs`
2. `desktop/host/src/main.rs`
3. `desktop/host/src/updater.rs`
4. `desktop/tauri-app/src-tauri/src/lib.rs`
5. `extension/src/lib/daemon-bridge.ts`
6. `extension/src/sw.ts`
7. `packages/client/src/host/host-channel.ts`
8. `packages/client/src/host/chrome-extension-channel.ts`
9. `packages/client/src/host/tauri-channel.ts`
10. `packages/client/src/components/SettingsOverlay.tsx`

## Verification
1. `source ~/.profile && cd desktop && cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace`
2. `pnpm run typecheck && pnpm run test && pnpm run lint && pnpm format:fix`
