# iOS Native Bindings Implementation Plan

## Purpose

Define a concrete path for bringing up iOS standalone support without starting in the simulator.

This plan focuses on:

- hardening the native binding contract
- building the host/runtime layer first
- validating behavior with XCTest on macOS and iOS
- deferring simulator/device work until the core binding surface is stable

## Guiding Decisions

| Decision | Choice | Why |
|----------|--------|-----|
| First runtime | **JavaScriptCore** | Built-in, lower packaging risk than QuickJS, good fit for early bring-up |
| Project generation | **`xcodegen`** | Reproducible project generation without hand-editing `.pbxproj` |
| First code shape | **Local Swift package + thin app target** | Most host code becomes testable without UI or simulator coupling |
| Contract style | **Lightweight contract + conformance cases** | High value without over-designing a schema system for binary payloads |
| Earliest test loop | **XCTest before simulator** | Faster iteration on bindings, packing, and engine bring-up |

## Non-Goals For Phase 1

- Full SwiftUI feature work
- Background execution
- Device-only network tuning
- App Store-safe remote-control architecture
- Replacing JavaScriptCore with QuickJS

## Why A Native Bindings Contract Is Worth It

The native binding layer is a fragile boundary:

- symbol names must match exactly
- argument and return shapes must match exactly
- result codes must stay aligned across hosts
- binary-packed batch formats must not drift
- async callback timing must stay consistent with engine expectations

This repo already uses explicit contract/conformance files for daemon and host boundaries in `contracts/`.
The iOS/native-runtime path should use the same pattern, but keep it lightweight.

## Contract Strategy

### Source of truth

Keep [`packages/engine/src/adapters/native/bindings.d.ts`](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/native/bindings.d.ts) as the developer-facing API reference.
Treat it as descriptive, not normative, when a host transport has compatibility quirks.

The machine-readable contract should be the normalized semantic source for:

- symbol direction:
  - JS calls native
  - native calls JS
  - JS-internal callback stores that batched dispatch depends on
- availability:
  - required
  - capability-gated
- logical return kinds, even when a host transport widens them
  - example: QuickJS may expose boolean success as `"true"` / `"false"`, but the contract should still describe boolean semantics

Add machine-readable companion files:

- `contracts/native-bindings-contract.json`
- `contracts/native-bindings-conformance.json`

### What goes in `native-bindings-contract.json`

This should describe:

- every required `__jstorrent_*` symbol
- symbol kind:
  - function
  - callback store
- symbol direction:
  - `js_calls_native`
  - `native_calls_js`
  - `js_internal_callback_store`
- availability:
  - `required`
  - `capability_gated`
- argument kinds: `string`, `number`, `boolean`, `arraybuffer`, `json_string`, `callback`
- return kinds: `void`, `string`, `number`, `boolean`, `arraybuffer`, `json_string`, `callback_store`
- async delivery mode where relevant:
  - direct return
  - callback registration
  - queued event flushed at tick boundary
- shared enums and codes:
  - write result codes
  - read result codes
  - verify chunk result bytes
- packed binary frame formats for:
  - TCP dispatch batches
  - UDP dispatch batches
  - file write result batches
  - file read result batches
  - hash result batches

This is intentionally not a full JSON Schema for every runtime behavior.
It is a practical contract description for validation and documentation.

### What goes in `native-bindings-conformance.json`

This should define named cases similar to the daemon conformance files, for example:

- `polyfill.text_roundtrip`
- `polyfill.random_bytes_length_matches`
- `hash.sha1_known_vector_matches`
- `hash.batch_packed_format_matches`
- `storage.get_set_delete_roundtrip`
- `file.stat_reports_metadata`
- `file.list_tree_reports_entries`
- `file.verify_chunks_match_mismatch_io_error`
- `file.verified_write_result_codes_match_contract`
- `file.async_read_batch_dispatch_matches_contract`
- `timer.timeout_fires_once`
- `timer.interval_repeats_until_cleared`
- `callbacks.state_update_shape_is_received`
- `tcp.batch_dispatch_routes_to_correct_socket`
- `udp.batch_dispatch_routes_to_correct_socket`
- `network.interfaces_shape_is_reported`
- `network.default_gateway_shape_is_reported`

The intent is not to force all hosts to implement everything on day one.
The intent is to make drift visible and staged bring-up explicit.

For iOS bring-up, network interface lookup, default gateway lookup, and multicast support should start as capability-gated rather than as bring-up blockers.

## Implementation Shape

### Repo structure

Initial target layout:

```text
ios/
├── project.yml
├── JSTorrent/
│   ├── App/
│   ├── Resources/
│   └── Support/
├── JSTorrentKit/
│   ├── Package.swift
│   ├── Sources/
│   │   └── JSTorrentKit/
│   │       ├── Engine/
│   │       ├── Bindings/
│   │       ├── Host/
│   │       ├── Contracts/
│   │       └── Models/
│   └── Tests/
│       └── JSTorrentKitTests/
└── JSTorrentAppTests/
```

### Package responsibilities

`JSTorrentKit` should own:

- `JSEngine`
- `EngineBundle`
- `EngineController`
- `NativeBindings`
- binding modules
- batch frame parsers/packers
- contract fixtures/helpers
- most tests

The app target should own:

- SwiftUI app shell
- resource bundling
- app lifecycle integration
- later entitlement/background configuration

## Test Strategy

### Layer 1: Pure unit tests

Runs without simulator and without loading the full engine.

Use this for:

- packed frame encoding/decoding
- write/read result code mapping
- `verify_chunks` semantics
- path normalization and root resolution
- timer bookkeeping
- callback registry behavior

These tests should mirror the style of Android's pure binding tests, especially the batch framing approach in [`FileBindingsTest.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/test/kotlin/com/jstorrent/quickjs/bindings/FileBindingsTest.kt).

### Layer 2: JSCore integration tests

Runs under XCTest, still without simulator-driven UI work.

Use this for:

- creating a `JSContext`
- registering Swift bindings
- loading `engine.bundle.js`
- executing JS snippets that call `__jstorrent_*`
- validating callback flow and batch flush behavior

This is the iOS equivalent of Android's runtime-facing binding tests in [`NativeBindingsTest.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/androidTest/kotlin/com/jstorrent/quickjs/NativeBindingsTest.kt), but much of it should stay package-testable on macOS.

### Layer 3: App target tests

Still avoid simulator-first development.
Use only for:

- resource lookup
- app-level controller wiring
- bundle loading

### Layer 4: Simulator and device tests

Do this after the binding surface is stable.

Use simulator for:

- app launch
- UI wiring
- resource bundling
- sandbox integration

Use real device for:

- UDP/DHT realism
- long-lived networking
- lifecycle transitions
- eventual background work

## Bring-Up Order

### Phase 0: Project and contract scaffolding

Deliverables:

- `xcodegen`-generated iOS project
- local `JSTorrentKit` package
- placeholder app target
- `contracts/native-bindings-contract.json`
- `contracts/native-bindings-conformance.json`
- initial contract validator/test helper

Acceptance criteria:

- project generates from checked-in config
- package tests run from command line
- contract files exist and validate structurally
- native engine init is parameterized for iOS instead of assuming Android

### Phase 1: JS engine host shell

Deliverables:

- `JSEngine.swift`
- serial JS execution queue
- script evaluation
- global function registration
- binary argument/return support
- engine bundle loader

Acceptance criteria:

- can load a simple JS snippet in XCTest
- can register native functions and call them from JS
- can pass `ArrayBuffer` data across the bridge

### Phase 2: Polyfills and callback backbone

Deliverables:

- text encode/decode
- random bytes
- console logging
- timers
- state/error callbacks
- shared callback dispatcher helpers

Acceptance criteria:

- text round-trip tests pass
- SHA1 test vectors pass
- timeout and interval tests pass
- state callback payload reaches Swift layer

### Phase 3: Storage, file sync ops, and hash ops

Deliverables:

- storage bindings
- sync file bindings
- sync hash bindings
- root resolution model for app-private storage

Acceptance criteria:

- storage CRUD passes in XCTest
- file stat/readdir/list-tree/read/write tests pass
- shared write error/result code mapping aligns with [`write-error.ts`](/Users/kgraehl/code/jstorrent/packages/engine/src/core/write-error.ts)

### Phase 4: Async file and hash batching

Deliverables:

- verified write batching
- async read batching
- hash result batching
- packed frame builders/parsers that match engine expectations

Acceptance criteria:

- `verify_chunks` behavior matches the daemon tests in [`daemon-filesystem.test.ts`](/Users/kgraehl/code/jstorrent/packages/engine/integration/daemon/daemon-filesystem.test.ts#L306)
- result batches decode correctly
- callback dispatch matches [`callback-manager.ts`](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/native/callback-manager.ts)

### Phase 5: Engine bootstrap

Deliverables:

- bundle copy/build step
- `jstorrent.init(...)` path works
- state subscription hookup
- host-driven tick loop

Acceptance criteria:

- engine loads under XCTest
- `jstorrent.init()` completes
- initial state callbacks are received
- no simulator required for basic bring-up

### Phase 6: TCP bindings

Deliverables:

- TCP connect/send/close
- callback registration
- queued inbound data
- tick-boundary flush batching
- TLS upgrade support

Acceptance criteria:

- local loopback socket tests pass
- packed TCP dispatch matches contract
- callback routing behaves like the native callback manager contract

### Phase 7: UDP bindings

Deliverables:

- UDP bind/send/close
- multicast join/leave
- queued inbound packet batching

Acceptance criteria:

- local UDP tests pass on host where supported
- batch dispatch shape matches contract
- simulator/device-only caveats are explicitly documented

### Phase 8: Network info bindings

Deliverables:

- network interfaces query
- default gateway query

Acceptance criteria:

- shape tests pass
- behavior is documented when gateway lookup is unavailable on a given runtime

### Phase 9: Minimal UI integration

Only after the runtime layer is stable.

Deliverables:

- minimal `EngineController` observable object
- placeholder torrent list UI
- add magnet command path

Acceptance criteria:

- app can launch and show engine state
- UI depends on an already-tested host layer

## Concrete First Milestone

The first meaningful milestone is not "app launches in simulator".
It is:

1. generate project from `xcodegen`
2. run `JSTorrentKit` XCTest bundle from command line
3. load JSCore
4. register polyfills and storage/file/hash bindings
5. load `engine.bundle.js`
6. call `jstorrent.init(...)`
7. receive a state callback

That milestone proves the host/runtime path is viable before any real UI work.

## Reference Implementations

Use these as the main references while implementing Swift bindings:

- Android binding facade: [`NativeBindings.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/NativeBindings.kt)
- Android file bindings: [`FileBindings.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/FileBindings.kt)
- Android TCP bindings: [`TcpBindings.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/TcpBindings.kt)
- Android UDP bindings: [`UdpBindings.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/UdpBindings.kt)
- Android polyfills: [`PolyfillBindings.kt`](/Users/kgraehl/code/jstorrent/android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/bindings/PolyfillBindings.kt)
- Native callback dispatch contract: [`callback-manager.ts`](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/native/callback-manager.ts)
- Shared write result codes: [`write-error.ts`](/Users/kgraehl/code/jstorrent/packages/engine/src/core/write-error.ts)
- Shared binding surface: [`bindings.d.ts`](/Users/kgraehl/code/jstorrent/packages/engine/src/adapters/native/bindings.d.ts)

## Immediate Next Steps

1. Install `xcodegen`.
2. Add the iOS project generator config and local Swift package scaffold.
3. Add the two native-binding contract files.
4. Keep networking extras capability-gated so iOS bring-up is not blocked on multicast or gateway discovery.
5. Implement `JSEngine` and the first XCTest target for JSCore smoke tests.
