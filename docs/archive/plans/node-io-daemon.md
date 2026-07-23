# Node I/O Daemon Reference Implementation

**Status:** Proposed
**Date:** 2026-03-10

See also:
- [node-daemon-client.md](node-daemon-client.md) - current Node client that talks to an external daemon
- [torrent-file-http-serving.md](torrent-file-http-serving.md) - HTTP media serving modes, including blocking `206`
- [../design/standalone-daemon-mode.md](../design/standalone-daemon-mode.md) - Rust standalone-daemon vision, including roots management
- [../../packages/docs/design-http-rpc-layer.md](/Users/kgraehl/code/jstorrent/packages/docs/design-http-rpc-layer.md) - current Node HTTP RPC orchestration layer constraints
- [../project/ARCHITECTURE.md](../project/ARCHITECTURE.md) - platform and daemon protocol overview

## Purpose

Define a separate Node-based daemon-compatible server that can act as a readable, test-friendly reference implementation of the external daemon surface used by the extension and other clients.

This is intentionally **not** the same thing as the current Node standalone engine runner / HTTP RPC harness.

## Why This Exists

Today we effectively have two daemon implementations:

- Rust desktop / standalone daemon
- Kotlin Android companion daemon

A third implementation in Node would be useful because it would be:

- easier to iterate on than Rust or Kotlin
- easier to inspect during protocol work
- useful as a test target for extension E2E and characterization tests
- a place to prototype new external surfaces like blocking `206`
- a way to expose spec drift between the Rust and Kotlin implementations

The main value is as a **reference implementation and test target**, not as the default production deployment.

## Naming and Separation

We should keep two Node entry points, with distinct purposes:

### 1. Node engine runner

This is the existing Node-side standalone engine process.

Characteristics:

- runs `BtEngine` directly with Node adapters
- useful as a CLI downloader, test harness, or orchestration surface
- may expose simple RPC endpoints for tests
- does **not** need to match the daemon-compatible protocol surface

Examples today:

- [packages/engine/src/node-rpc/server.ts](/Users/kgraehl/code/jstorrent/packages/engine/src/node-rpc/server.ts)
- [docs/plans/node-daemon-client.md](node-daemon-client.md)

### 2. Node I/O daemon

This is the new thing proposed here.

Characteristics:

- separate from the simple Node RPC harness
- exposes the same external daemon-compatible surface that clients expect
- intended to be usable by the extension without a special client mode
- acts as a spec anchor and reference implementation

This document is about the second one.

## Relationship to `node-daemon-client`

The existing `node-daemon-client` is important here.

It already fills a role similar to the extension's engine-side client behavior:

- it runs `BtEngine`
- it connects to an external daemon
- it consumes the daemon-facing protocol surface rather than implementing it

That makes it a natural Node-side client peer for the proposed `node-io-daemon`.

In other words:

- `node-daemon-client` is the Node reference for the client/consumer side
- `node-io-daemon` is the Node reference for the daemon/server side

Together they create a useful local integration harness that does not require the browser extension runtime.

This is valuable for:

- protocol smoke tests
- characterization tests
- compatibility checks during protocol changes
- validating new daemon features before involving Rust, Kotlin, or the extension runtime

## Goals

- Provide a daemon-compatible Node server that speaks the same externally visible protocol shape as the Kotlin and Rust daemons.
- Make it easy to run in tests and local development.
- Support the extension as a client with minimal or no extension-side special cases.
- Include download roots management so it is a full reference surface, not just a partial fake.
- Serve as the first proving ground for new daemon-facing features when that reduces iteration cost.

## Non-Goals

- Replace the Rust desktop daemon in shipping desktop builds.
- Replace the Android companion on ChromeOS or Android.
- Force `BtEngine` to know anything about daemon orchestration or HTTP/WebSocket protocol details.
- Reproduce the desktop native-host + io-daemon multi-process topology exactly.

For this reference implementation, a single Node process is fine.

## Architectural Position

The Node I/O daemon should sit at the same architectural layer as the Rust and Kotlin daemons:

```
extension / website / test client
            |
            | daemon-compatible protocol
            v
      node-io-daemon
            |
            | internal calls
            v
         BtEngine
```

Important distinction:

- The external surface should look like a daemon.
- Internally, it can call Node adapters or `BtEngine` directly.

It does **not** need to proxy to a second local Node process unless we later decide that process separation materially improves tests.

## Required Compatibility Surface

To be a useful reference implementation, this server should eventually implement the following surfaces.

### HTTP bootstrap and discovery

- `GET /health`
- `/status`
- pairing or test-mode auth bootstrap
- capability reporting where applicable

This is needed so the extension can find it and decide how to connect.

### WebSocket `/io`

- hello/auth handshake
- binary envelope format
- request/response correlation
- TCP socket operations
- UDP socket operations
- server listen/accept operations
- any control opcodes currently expected on the data plane

This is the most important compatibility layer.

### Control/event surface

- roots-changed notifications
- native event forwarding
- any control-plane messages the extension relies on today

Whether this is a separate `/control` socket or folded into the existing model should follow the current client expectations closely enough for real integration tests.

### File and utility endpoints

- file read / write endpoints used by daemon-backed storage
- hashing endpoints as needed
- any small helper HTTP endpoints relied on by daemon clients

### Roots management

- list roots
- add root
- remove root
- persist roots locally
- choose sensible default root behavior in local/test runs

Roots are important. Without them, this becomes only a partial protocol fake rather than a serious reference implementation.

### Media serving surfaces

Eventually:

- complete-file `206`
- blocking torrent-aware `206`
- possibly HLS HTTP endpoints if that becomes part of the daemon surface

These are not required for the very first phase, but they are a strong reason to have this implementation.

## Where It Should Reuse Existing Code

The Node I/O daemon should reuse:

- `BtEngine`
- Node filesystem/socket/hash adapters
- existing torrent/session/storage logic
- any protocol packing/unpacking helpers that can be shared cleanly

It should **not** reuse the current Node RPC layer as-is as its public interface, because that layer is intentionally a simple orchestration scaffold rather than a daemon-compatible contract.

That said, some pieces are still useful:

- process startup patterns
- engine lifecycle management
- test harness conveniences
- pieces of the new complete-file range endpoint if we keep that functionality on the Node side

## Where It Should Stay Separate

The current Node HTTP RPC server should remain a separate tool.

Reasons:

- it is useful as a simple CLI/test runner on its own
- its API is intentionally not daemon-compatible
- its design docs explicitly treat it as orchestration, not as an engine-internal or daemon-compat surface
- mixing the two concerns would make both harder to reason about

Recommended split:

- `node-rpc/` for engine orchestration
- new `node-io-daemon/` area for daemon-compatible serving

Exact path can be decided later, but the conceptual split should be maintained.

## Test Matrix Value

One of the main reasons to build `node-io-daemon` is to create a better compatibility matrix.

Examples:

- extension ↔ Rust daemon
- extension ↔ Kotlin daemon
- extension ↔ Node I/O daemon
- node-daemon-client ↔ Rust daemon
- node-daemon-client ↔ Kotlin daemon
- node-daemon-client ↔ Node I/O daemon

The last case is especially useful early on:

- it is fully local and scriptable
- it avoids browser/runtime complications
- it gives us a client/server integration test path in a single language

That should be one of the first validation targets as `node-io-daemon` becomes capable enough to support it.

## Auth Model

There are two acceptable modes:

### Test mode

- fixed token or auto-generated local token
- auto-approve pairing
- optimized for local integration tests

### Realistic mode

- match current daemon auth shape closely enough to exercise the extension handshake flow
- preserve extension identity concepts if they matter to client behavior

The reference implementation should support at least test mode first.

## Roots Management

The Node I/O daemon should include local roots persistence from the start or very early.

Suggested minimal behavior:

- default root points at a local writable directory
- roots stored in a small local config file
- `GET /roots` returns current roots
- `POST /roots` adds/removes roots
- emit roots-changed notifications to connected clients

This aligns with the desired standalone-daemon direction described in [standalone-daemon-mode.md](../design/standalone-daemon-mode.md).

## Suggested Rollout Phases

### Phase 0: Scaffolding

- create a separate Node daemon package/module area
- keep it distinct from the current Node RPC harness
- define config, lifecycle, and test-mode auth

### Phase 1: Discovery and bootstrap

- `GET /health`
- `/status`
- minimal auth/bootstrap behavior
- enough for a client to detect and identify the server

Validation target:

- add a first `node-daemon-client ↔ node-io-daemon` smoke test once this phase is complete

### Phase 2: `/io` handshake and envelope compatibility

- implement hello/auth
- implement frame packing/unpacking
- implement request correlation
- stand up the basic socket manager structure

This is the first phase where the extension can start talking to it in a meaningful way.
It is also the phase where `node-daemon-client ↔ node-io-daemon` becomes a meaningful protocol integration test instead of just a process-liveness check.

### Phase 3: Core socket ops

- TCP connect/send/recv/close
- TCP listen/accept
- UDP bind/send/recv/close
- whatever else is minimally required for the engine to operate through the daemon surface

### Phase 4: Roots and control events

- roots persistence
- roots-changed notifications
- event forwarding
- capability reporting

### Phase 5: File/hash endpoints

- daemon-backed filesystem reads/writes
- hashing endpoints
- enough to support the daemon-backed storage path without major client conditionals

### Phase 6: Media serving

- complete-file `206`
- blocking torrent-aware `206`
- any follow-on HTTP media surfaces we decide belong here

At that point, `node-daemon-client` and extension-based tests can both exercise those surfaces against the same reference daemon.

## Current Node Blocking Stream Contract

The Node implementation is now far enough along to use as the leading-edge contract for Kotlin and Rust blocking `/stream/{token}` work.

Current registration/session shape:

- `REGISTER_HTTP_STREAM` is torrent-aware and carries `torrentId` plus `fileIndex`.
- The daemon stores tokenized stream records tied to torrent lifecycle and control-session ownership.
- The media server exposes real `GET` and `HEAD` handling on `/stream/{token}` with standard HTTP `Range` parsing.

Current byte-serving contract:

- The daemon waits chunk-by-chunk, not whole-file-at-once.
- The current daemon chunk size is `256 KiB`.
- For each chunk, the daemon asks the engine bridge to `waitForRange(offset, length)` and then reads bytes from disk normally.
- Already-complete byte ranges serve directly even if the torrent later becomes stopped or the file later becomes skipped.

Current lifecycle/error contract:

- `torrent removed`
  - revoke all tokens for that torrent
  - cancel all in-flight waits
  - new requests return `404`
- `torrent stopped`
  - keep tokens alive
  - serve already-complete ranges
  - fail incomplete ranges quickly with `409`
  - cancel in-flight waits
- `file skipped`
  - serve already-complete ranges
  - fail incomplete ranges quickly with `409`
- `torrent queued/inactive`
  - fail incomplete ranges quickly with `409`
- `torrent error state`
  - fail incomplete ranges quickly with `409`

Current concurrency contract:

- One token may serve multiple concurrent HTTP readers.
- Canceling one request must not cancel a different request on the same token.
- Torrent stop/remove must fan out to every active reader on that token.
- Mixed outcomes are allowed:
  - one request may get `206` for an already-complete range
  - another concurrent request on the same token may get `409` if it still needs missing bytes

Current Node status vocabulary:

- `FileSkipped`
- `TorrentErrored`
- `TorrentInactive`
- `TorrentRemoved`
- `TorrentStopped`
- `StreamSessionNotFound`
- `StreamSessionMismatch`

Those names now exist as shared Node constants and are the recommended starting vocabulary for Kotlin/Rust parity work, even if the wire shape later becomes typed result objects instead of raw string identifiers.

## Why This Is Valuable Even If It Never Ships Broadly

- It gives us a spec-checking implementation that is easier to modify than Rust/Kotlin.
- It gives extension tests a daemon target we can run locally and instrument heavily.
- It helps isolate whether bugs are in the extension client, the daemon contract, or a runtime-specific implementation.
- It makes protocol changes safer by providing a third implementation during migration windows.
- It gives us a clean place to prototype daemon-facing media endpoints before hardening them elsewhere.

## Risks

### Risk: accidental divergence from real daemons

Mitigation:

- treat protocol compatibility as the primary goal
- use characterization tests against current clients
- keep explicit parity checklists

### Risk: turning into a second ad hoc Node harness

Mitigation:

- keep it separate from the existing `node-rpc` orchestration layer
- define compatibility requirements up front
- prefer extension-facing tests over ad hoc bespoke clients

### Risk: too much implementation cost for limited value

Mitigation:

- build in phases
- start with the surfaces that unlock extension integration tests
- delay media serving and other advanced features until the core daemon surface proves useful

## Open Questions

- Should the Node I/O daemon expose exactly the same endpoints and routes as Rust/Kotlin, or allow a small amount of Node-only flexibility so long as extension tests use the common path?
- Should pairing be implemented fully, or should test mode be the default with full pairing optional?
- Should roots config live in a dedicated config file, or reuse any existing Node session/config location?
- Should the Node I/O daemon eventually support the standalone website client directly, or should it stay focused on extension/test compatibility?
- How much protocol code can be shared cleanly across runtimes without coupling the engine to transport concerns?

## Recommended Immediate Next Step

Create the scaffolding and parity checklist before writing much code:

1. define the module layout for `node-io-daemon`
2. list the exact compatibility endpoints/opcodes required for extension bootstrap
3. implement `GET /health` and `/status`
4. add a first extension-side integration test that targets the Node I/O daemon in a controlled environment

That will let us prove the concept with minimal commitment while keeping the long-term separation clean.
