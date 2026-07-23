# Explicit Launch Model

## Goal

Make launch and attachment decisions explicit instead of spreading them across UI events, deep-link handlers, bridge bootstrap code, and platform-specific side effects.

This document does two things:

1. Inventory the launch and activation paths encoded today.
2. Define the explicit launch contract that future offscreen, service worker, desktop, ChromeOS, Android, and remote-control paths should use.

## Current Model

Today the system has several launch paths, but they are implicit:

- an externally connectable page can host the UI and talk through the extension
- opening the extension UI starts or resumes the host path
- desktop eagerly bootstraps the native host from the service worker
- website `launch-ping` opens UI and forwards add-torrent work
- desktop deep links route either to Tauri or to the extension
- ChromeOS launch requires an intent/bootstrap flow
- desktop takeover and profile switching are encoded as separate control actions

There is no single launch request object that answers:

- who requested launch
- which profile should own the engine
- where the engine should run
- where the I/O backend should run
- how those targets may be activated
- whether the system should attach to an existing engine or start a new one

## Current Implicit Launch Paths

### 0. External Page Direct Attach

Current trigger:

- an approved external page runs the client UI directly and connects to the extension

Current encoding:

- `ChromeExtensionChannel` has an external mode that sends messages to a specific extension ID
- the service worker accepts external `ui` ports and routes them through the same UI port handler
- this is distinct from `launch-ping`: the page itself hosts the engine/UI instead of asking the extension to open its own UI

Relevant code:

- [packages/client/src/host/create-host-channel.ts](/Users/kgraehl/code/jstorrent/packages/client/src/host/create-host-channel.ts)
- [packages/client/src/host/chrome-extension-channel.ts](/Users/kgraehl/code/jstorrent/packages/client/src/host/chrome-extension-channel.ts)
- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)
- [extension/public/manifest.json](/Users/kgraehl/code/jstorrent/extension/public/manifest.json)

Implicit meaning:

- requester = `website`
- engine target = `external_page`
- backend target = whichever backend the external page eventually reaches through the extension
- activation = `attach_existing`

Examples currently allowlisted:

- `https://jstorrent.com/*`
- `https://playsvideo.com/*`

Important distinction:

- `jstorrent.com` is the current concrete example of this path
- it explicitly hosts the engine in the page and uses the extension to reach I/O/backend capabilities
- `playsvideo.com` is allowlisted, but the product goal for it is different and not fully implemented yet

This current direct-host path should be modeled separately from `launch-ping`.

### 1. Extension UI Open

Current trigger:

- user opens the extension UI

Current encoding:

- UI port connect clears the idle timer and reconnects the bridge if disconnected
- service worker remains the broker
- engine still lives in the foreground UI page

Relevant code:

- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)

Key lines:

- `handleUIPortConnect()` reconnects the bridge on UI attach

Implicit meaning:

- requester = `extension_ui`
- engine target = `extension_foreground`
- backend target = `browser_local` initially, then whatever backend the bridge connects to
- activation = `start_if_needed`

### 2. Desktop Eager Bridge Bootstrap

Current trigger:

- service worker startup on desktop

Current encoding:

- desktop calls `bridge.connect()` eagerly at top level
- this bootstraps native-host connectivity before UI opens

Relevant code:

- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)

Implicit meaning:

- requester = `system`
- engine target = `extension_foreground`
- backend target = `desktop_native_stack`
- activation = `background_launch`

This is backend bootstrap, not a clean engine launch contract.

### 3. Website Launch Ping

Current trigger:

- approved external site sends `launch-ping`

Current encoding:

- service worker validates token if present
- magnet/torrent payload is buffered
- UI tab is opened
- add work is completed through the current UI/bridge path

Relevant code:

- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)

Implicit meaning:

- requester = `website`
- engine target = `extension_foreground`
- backend target = whichever backend the foreground engine eventually attaches to
- activation = `start_if_needed`

This is closer to "open the extension-owned path for me" than to "let the website acquire the best available engine."

### 4. Launch Desktop App

Current trigger:

- extension UI or service worker sends `LAUNCH_DESKTOP`

Current encoding:

- service worker forwards to native host
- native host launches Tauri desktop app

Relevant code:

- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)
- [extension/src/lib/daemon-bridge.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/daemon-bridge.ts)

Implicit meaning:

- requester = `extension_ui`
- engine target = `desktop_app`
- backend target = `desktop_native_stack`
- activation = `launch_desktop_app`

### 5. ChromeOS Trigger Launch

Current trigger:

- user uses ChromeOS bootstrap flow

Current encoding:

- extension opens Android intent URL
- bootstrap polls until companion becomes reachable
- daemon bridge connects after bootstrap succeeds

Relevant code:

- [extension/src/lib/chromeos-bootstrap.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/chromeos-bootstrap.ts)
- [extension/src/lib/daemon-bridge.ts](/Users/kgraehl/code/jstorrent/extension/src/lib/daemon-bridge.ts)

Implicit meaning:

- requester = `extension_ui`
- engine target = `extension_foreground`
- backend target = `chromeos_android_companion`
- activation = `user_gesture_intent`

Important correction:

- the Android companion is not where the torrent engine runs today
- it is the backend/native-I/O side that the extension-hosted engine talks to
- Crostini is a similar shape: extension-hosted engine, Crostini-hosted backend

### 6. Tauri Deep-Link Local Route

Current trigger:

- OS deep link reaches the desktop app

Current encoding:

- Tauri decides whether to handle locally or route to extension
- if desktop window is visible, it handles locally and shows desktop UI

Relevant code:

- [desktop/tauri-app/src-tauri/src/lib.rs](/Users/kgraehl/code/jstorrent/desktop/tauri-app/src-tauri/src/lib.rs)

Implicit meaning:

- requester = `desktop_deeplink`
- engine target = `desktop_app`
- backend target = `desktop_native_stack`
- activation = `attach_existing`

### 7. Tauri Deep-Link Route To Extension

Current trigger:

- OS deep link reaches the desktop app while policy prefers the extension

Current encoding:

- Tauri opens the extension launch URL
- extension then handles the request through `launch-ping`

Relevant code:

- [desktop/tauri-app/src-tauri/src/lib.rs](/Users/kgraehl/code/jstorrent/desktop/tauri-app/src-tauri/src/lib.rs)
- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)

Implicit meaning:

- requester = `desktop_deeplink`
- engine target = `extension_foreground`
- backend target = whichever backend the extension path currently resolves to
- activation = `handoff_to_extension`

### 8. Profile Switch / TakeOver

Current trigger:

- user switches profile or requests takeover

Current encoding:

- extension profile switch writes `profileId`, disconnects, reconnects
- desktop takeover kills incumbent and re-handshakes

Relevant code:

- [extension/src/sw.ts](/Users/kgraehl/code/jstorrent/extension/src/sw.ts)
- [desktop/host/src/main.rs](/Users/kgraehl/code/jstorrent/desktop/host/src/main.rs)

Implicit meaning:

- these are ownership transfer paths, but they also behave like launch paths

## Current Gaps

The current model has several issues:

- launch intent is encoded in side effects rather than data
- profile ownership and engine ownership are not represented as first-class records
- engine placement and backend placement are conflated in some paths
- some paths launch UI, some launch backends, and some only enqueue work
- website, deep-link, desktop, and ChromeOS flows are hard to compare
- service worker lifecycle, engine lifecycle, and backend lifecycle are coupled indirectly
- testing mostly validates individual code paths, not a shared launch contract

## Near-Term Product Driver

The main product driver for this refactor is not the existing `jstorrent.com` direct-host path.

It is a future `playsvideo.com` acquisition flow with different behavior:

- first, discover whether a suitable engine is already running
- if yes, attach to it
- if not, start the best available engine/runtime
- prefer background-capable placements when possible, such as `extension_offscreen`
- fall back to foreground or other supported targets when needed

That desired flow is not fully implemented today.

### Current `jstorrent.com` vs Planned `playsvideo.com`

`jstorrent.com` today:

- hosts the engine/UI in the page
- talks through the extension
- is closer to `engineTarget = external_page`

`playsvideo.com` target behavior:

- should acquire whatever engine is already available
- should be able to request a background-capable engine if none is running
- should not be forced into the same direct-host model as `jstorrent.com`
- is the main reason to formalize attach-vs-start-vs-fallback decisions

This difference is why the explicit launch contract needs to model:

- current engine ownership
- candidate engine targets
- candidate backend targets
- attach-first behavior
- fallback order

## Proposed Explicit Model

The system should move to a small explicit vocabulary:

- `EngineTarget`: where the torrent engine should run
- `IoBackendTarget`: where native socket/filesystem/proxy work should run
- `ControlBroker`: which runtime brokers launch/control
- `LaunchRequester`: who asked for it
- `ActivationPath`: how the target may be activated
- `StartMechanism`: what primitive is actually available to start or attach
- `LaunchPolicyResult`: whether that activation is allowed
- `EngineInstanceRecord`: which profile currently owns a running engine

### Core Concepts

Profiles:

- durable identity
- own KV/session scope
- may be selected, switched, or taken over

Engine instances:

- runtime owner for a single profile
- one active owner per `profileId`
- UI, websites, and remote clients attach to an owner instead of creating duplicates

Engine targets:

- `external_page`
- `extension_foreground`
- `extension_offscreen`
- `extension_service_worker`
- `desktop_app`
- `ios_app`
- `android_app`

I/O backend targets:

- `browser_local`
- `desktop_native_stack`
- `ios_native_stack`
- `chromeos_android_companion`
- `chromeos_crostini_daemon`
- `android_native_stack`

Control brokers:

- `chrome_extension`
- `safari_extension`
- `web_page`
- `desktop_app`
- `ios_app`
- `android_app`

Start mechanisms:

- `browser_native_host_autostart`
- `extension_message_attach`
- `intent_url`
- `os_deeplink`
- `os_file_association`
- `manual_launch`
- `attach_only`

Activation paths:

- `attach_existing`
- `start_if_needed`
- `background_launch`
- `launch_desktop_app`
- `user_gesture_intent`
- `handoff_to_extension`
- `push_wake`
- `persistent_remote_control`
- `manual_launch`

## Why These Need To Be Separate

The same engine/backend pair can have very different launch properties depending
on browser and platform.

Examples:

Desktop Chrome with native host:

- broker = `chrome_extension`
- backend = `desktop_native_stack`
- start mechanism = `browser_native_host_autostart`

This is the strongest current cold-start path because the browser can launch the
native host directly.

ChromeOS extension + Android companion:

- broker = `chrome_extension`
- engine target = `extension_foreground`
- backend = `chromeos_android_companion`
- start mechanism = `intent_url` or attach-to-running

Here the extension is useful mainly as a secure communication and pairing broker,
not because it has desktop-style native-host autostart.

Safari:

- broker is not `chrome_extension`
- desktop native-host autostart is not available through the Chrome extension path
- startup tends to rely on `os_deeplink`, `os_file_association`, `manual_launch`, or attach-only behavior

That is why `engineTarget` and `backendTarget` are not enough on their own.
They tell us placement, but not who is coordinating launch or what start
primitive is available in the current runtime.

## Migration Strategy

### Phase 1. Document And Encode

- add a shared launch vocabulary in TypeScript
- encode the currently known implicit launch paths
- do not change runtime behavior yet

### Phase 2. Centralize Decisions

- add a service-worker launch supervisor
- make current paths construct a `LaunchRequest`
- keep the existing platform-specific side effects behind adapters
- add an attach-first acquisition flow for future `playsvideo.com` behavior

### Phase 3. Add Engine Registry

- track engine ownership by `profileId`
- surface `listProfiles()` and `listEngines()`
- distinguish attach vs start vs transfer
- record both `engineTarget` and `backendTarget`

### Phase 4. Add New Targets

- add `extension_offscreen`
- add optional `extension_service_worker`
- add remote wake requesters and activation policies

## Testing Strategy

This change needs layered testing because reality is system-integrated.

### 1. Pure Policy Tests

Test launch decisions as data:

- input: `LaunchRequest` + current registry state + platform state
- output: `LaunchDecision`

This should cover:

- attach vs start
- profile conflict
- target fallback
- gesture-required paths
- remote wake allowed/blocked

### 2. Adapter Tests

Test each launch adapter independently:

- extension UI attach/open
- website `launch-ping`
- desktop native launch
- ChromeOS intent bootstrap
- Tauri deep-link handoff

### 3. Small E2E Smoke Tests

Keep the matrix intentionally small:

- website to extension foreground
- extension to desktop app
- ChromeOS gesture to companion
- profile takeover

### 4. Runtime Observability

Persist structured launch records:

- requester
- target
- activation
- profileId
- outcome
- error

That should become the primary reality check when the actual system diverges from the intended model.

## Immediate Next Step

The first implementation step is not offscreen launch itself.

It is:

1. create the explicit launch vocabulary
2. map the current implicit paths into that vocabulary, including separate engine and backend placement
3. route future launch work through one supervisor API

That preserves current behavior while making new launch targets additive instead of ad hoc.

## Appendix: Draft Type Vocabulary

The first draft of this vocabulary previously lived in
`extension/src/lib/launch-model.ts`. It is kept here for now because it is
documentation-oriented and does not participate in runtime behavior yet.

```ts
export type EngineTarget =
  | 'external_page'
  | 'extension_foreground'
  | 'extension_offscreen'
  | 'extension_service_worker'
  | 'desktop_app'
  | 'ios_app'
  | 'android_app'

export type IoBackendTarget =
  | 'none'
  | 'browser_local'
  | 'desktop_native_stack'
  | 'ios_native_stack'
  | 'chromeos_android_companion'
  | 'chromeos_crostini_daemon'
  | 'android_native_stack'

export type ControlBroker =
  | 'chrome_extension'
  | 'safari_extension'
  | 'web_page'
  | 'desktop_app'
  | 'ios_app'
  | 'android_app'
  | 'none'

export type LaunchRequester =
  | 'system'
  | 'extension_ui'
  | 'website'
  | 'desktop_deeplink'
  | 'chromeos_intent'
  | 'android_intent'
  | 'remote_control'

export type ActivationPath =
  | 'attach_existing'
  | 'start_if_needed'
  | 'background_launch'
  | 'launch_desktop_app'
  | 'user_gesture_intent'
  | 'handoff_to_extension'
  | 'push_wake'
  | 'persistent_remote_control'
  | 'manual_launch'

export type StartMechanism =
  | 'browser_native_host_autostart'
  | 'extension_message_attach'
  | 'intent_url'
  | 'os_deeplink'
  | 'os_file_association'
  | 'manual_launch'
  | 'attach_only'

export type LaunchPolicyResult =
  | 'allowed'
  | 'requires_user_gesture'
  | 'requires_approval'
  | 'blocked'

export type EngineInstanceStatus = 'starting' | 'running' | 'stopped' | 'error'

export interface LaunchRequest {
  profileId: string | null
  requester: LaunchRequester
  engineTarget: EngineTarget
  backendTarget: IoBackendTarget
  broker?: ControlBroker
  startMechanism?: StartMechanism
  activation: ActivationPath
  reason: string
  fallbackEngineTargets?: EngineTarget[]
  fallbackBackendTargets?: IoBackendTarget[]
}

export interface LaunchDecision {
  request: LaunchRequest
  policy: LaunchPolicyResult
  effectiveEngineTarget: EngineTarget
  effectiveBackendTarget: IoBackendTarget
  effectiveBroker?: ControlBroker
  effectiveStartMechanism?: StartMechanism
  effectiveActivation: ActivationPath
  reason?: string
}

export interface EngineInstanceRecord {
  profileId: string
  engineTarget: EngineTarget
  backendTarget: IoBackendTarget
  status: EngineInstanceStatus
  visible: boolean
  startedAt: number
  requester: LaunchRequester
}

export interface CurrentImplicitLaunchPath {
  kind: 'engine_launch' | 'backend_bootstrap' | 'ownership_transfer'
  id:
    | 'extension_ui_open'
    | 'external_page_direct_attach'
    | 'desktop_eager_bridge_bootstrap'
    | 'website_launch_ping'
    | 'launch_desktop'
    | 'chromeos_trigger_launch'
    | 'chromeos_crostini_attach'
    | 'tauri_deeplink_local'
    | 'tauri_deeplink_to_extension'
    | 'profile_transfer'
  requester: LaunchRequester
  engineTarget: EngineTarget
  backendTarget: IoBackendTarget
  activation: ActivationPath
  summary: string
}

export interface PlannedLaunchScenario {
  id: 'playsvideo_managed_engine_acquire'
  requester: LaunchRequester
  preferredEngineTargets: EngineTarget[]
  preferredBackendTargets: IoBackendTarget[]
  summary: string
}

export const CURRENT_IMPLICIT_LAUNCH_PATHS: CurrentImplicitLaunchPath[] = [
  {
    kind: 'engine_launch',
    id: 'external_page_direct_attach',
    requester: 'website',
    engineTarget: 'external_page',
    backendTarget: 'browser_local',
    activation: 'attach_existing',
    summary:
      'An externally connectable page such as jstorrent.com can host the engine/UI itself and relay control through the extension via external messaging and ports.',
  },
  {
    kind: 'engine_launch',
    id: 'extension_ui_open',
    requester: 'extension_ui',
    engineTarget: 'extension_foreground',
    backendTarget: 'browser_local',
    activation: 'start_if_needed',
    summary:
      'Opening the extension UI implicitly starts or resumes the foreground engine path, which today may later attach to a native backend.',
  },
  {
    kind: 'backend_bootstrap',
    id: 'desktop_eager_bridge_bootstrap',
    requester: 'system',
    engineTarget: 'extension_foreground',
    backendTarget: 'desktop_native_stack',
    activation: 'background_launch',
    summary: 'Desktop service worker startup eagerly bootstraps native-host connectivity.',
  },
  {
    kind: 'engine_launch',
    id: 'website_launch_ping',
    requester: 'website',
    engineTarget: 'extension_foreground',
    backendTarget: 'browser_local',
    activation: 'start_if_needed',
    summary:
      'External launch-ping opens the extension UI and routes magnet or torrent payloads into the extension foreground-engine path.',
  },
  {
    kind: 'engine_launch',
    id: 'launch_desktop',
    requester: 'extension_ui',
    engineTarget: 'desktop_app',
    backendTarget: 'desktop_native_stack',
    activation: 'launch_desktop_app',
    summary: 'Extension requests that the native host launch the Tauri desktop app.',
  },
  {
    kind: 'engine_launch',
    id: 'chromeos_trigger_launch',
    requester: 'extension_ui',
    engineTarget: 'extension_foreground',
    backendTarget: 'chromeos_android_companion',
    activation: 'user_gesture_intent',
    summary:
      'ChromeOS opens an Android intent and waits for bootstrap so the extension foreground engine can use the companion backend.',
  },
  {
    kind: 'engine_launch',
    id: 'chromeos_crostini_attach',
    requester: 'extension_ui',
    engineTarget: 'extension_foreground',
    backendTarget: 'chromeos_crostini_daemon',
    activation: 'start_if_needed',
    summary:
      'Crostini is the partially supported ChromeOS variant where the extension foreground engine uses a Crostini-hosted backend instead of the Android companion.',
  },
  {
    kind: 'engine_launch',
    id: 'tauri_deeplink_local',
    requester: 'desktop_deeplink',
    engineTarget: 'desktop_app',
    backendTarget: 'desktop_native_stack',
    activation: 'attach_existing',
    summary: 'Desktop deep links are handled locally when Tauri is already the active surface.',
  },
  {
    kind: 'engine_launch',
    id: 'tauri_deeplink_to_extension',
    requester: 'desktop_deeplink',
    engineTarget: 'extension_foreground',
    backendTarget: 'browser_local',
    activation: 'handoff_to_extension',
    summary: 'Desktop deep links may hand off to the extension launch page instead of local UI.',
  },
  {
    kind: 'ownership_transfer',
    id: 'profile_transfer',
    requester: 'extension_ui',
    engineTarget: 'desktop_app',
    backendTarget: 'desktop_native_stack',
    activation: 'attach_existing',
    summary:
      'Profile switch and takeover flows transfer ownership by reconnecting or replacing an incumbent.',
  },
]

export const PLANNED_LAUNCH_SCENARIOS: PlannedLaunchScenario[] = [
  {
    id: 'playsvideo_managed_engine_acquire',
    requester: 'website',
    preferredEngineTargets: [
      'external_page',
      'extension_offscreen',
      'extension_foreground',
      'extension_service_worker',
    ],
    preferredBackendTargets: [
      'desktop_native_stack',
      'chromeos_android_companion',
      'chromeos_crostini_daemon',
      'browser_local',
    ],
    summary:
      'playsvideo.com is the near-term product driver: it should attach to any suitable running engine first, and if none exists, request that the extension start the best available background-capable engine/runtime.',
  },
]
```
