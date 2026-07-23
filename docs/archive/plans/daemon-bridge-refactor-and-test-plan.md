# DaemonBridge Refactor + Test Plan

## Goal
Refactor `extension/src/lib/daemon-bridge.ts` incrementally while improving confidence and preserving behavior.

## Current Problem
`DaemonBridge` currently combines multiple concerns in one class:
- Desktop native messaging lifecycle and request/response flow
- ChromeOS HTTP + WebSocket pairing/connection flow
- Shared protocol/frame parsing and root mapping
- State store/listener/event management

This makes targeted changes risky and test setup expensive.

## Target Module Layout
1. Keep `extension/src/lib/daemon-bridge.ts` as a stable facade and singleton export.
2. Add `extension/src/lib/daemon-bridge/types.ts` for shared state/event types and constants.
3. Add `extension/src/lib/daemon-bridge/state-store.ts` for state updates, listeners, and event fanout.
4. Add `extension/src/lib/daemon-bridge/protocol/control-frame.ts` for frame build/parse helpers.
5. Add `extension/src/lib/daemon-bridge/protocol/root-mapper.ts` for Android/Crostini root normalization.
6. Add `extension/src/lib/daemon-bridge/desktop/desktop-connector.ts` for desktop handshake/native ops/takeover.
7. Add `extension/src/lib/daemon-bridge/chromeos/chromeos-connector.ts` for ChromeOS discovery/pairing/ws control ops.
8. Add `extension/src/lib/daemon-bridge/shared/health-check.ts` for interval lifecycle.

## Public Facade Contract to Preserve
Call sites in `extension/src/sw.ts` currently depend on:
- `connect`, `disconnect`, `getState`, `getPlatform`, `subscribe`, `onEvent`
- `hasEverConnected`, `getLastConnectedTime`, `getStats`
- `triggerLaunch`, `takeOver`, `launchDesktop`, update/profile operations
- `isDesktopHost`, `isAndroidCompanion`
- `sendKvRequest`, `sendNativeKvRequest`

This API remains stable through extraction.

## Incremental Refactor Sequence
1. Add characterization tests around current public behavior (no code move).
2. Extract pure helpers first (`root-mapper`, `control-frame`) and test directly.
3. Extract Desktop connector and route facade calls to it.
4. Extract ChromeOS connector and route facade calls to it.
5. Extract shared state store + health checker.
6. Remove dead private methods from facade after parity is proven.

Each extraction step must keep characterization tests green.

## Test Strategy

### P0 Characterization (first)
- `connect()` de-duplicates concurrent calls and performs one handshake path.
- `connect()` short-circuits when already connected.
- `disconnect()` clears connected state fields.
- Successful connect persists `daemon:hasConnectedSuccessfully` and `daemon:lastConnectedTime`.

### P0 Connector Behavior
- Desktop handshake success updates state and stores `profileId` when present.
- Desktop `profile_in_use` surfaces `lastError` and `profileInUseInfo`.
- ChromeOS paired `/status` + `/roots` + ws auth reaches connected.
- ChromeOS unpaired Android path requires `triggerLaunch()`.

### P1 Protocol/Helpers
- Root mapping handles `uri/path`, `displayName/display_name`, and stat/disk aliases.
- Control frame encoding uses expected header fields and little-endian requestId.
- KV/control response correlation by requestId resolves/rejects pending promises.

### P1 Lifecycle
- Health check clears prior interval before starting a new one.
- Failed health check transitions to disconnected and triggers reconnect policy on desktop.

## Test Harness Additions
Add reusable test helpers in `extension/test/helpers/`:
- `mock-chrome-full.ts` (runtime, storage, tabs, connectNative)
- `mock-native-port.ts` (listener registration + emit helpers)
- `mock-websocket.ts` (controllable ws lifecycle)
- `mock-fetch-router.ts` (route-driven fetch responses)

## Execution Plan (Now)
Phase 1 implementation in this task:
1. Add the helper modules above.
2. Add initial P0 characterization suite for `DaemonBridge`.
3. Run extension unit tests and fix regressions.

Subsequent tasks:
1. Extract `root-mapper` + `control-frame`.
2. Extract Desktop connector.
3. Extract ChromeOS connector.
