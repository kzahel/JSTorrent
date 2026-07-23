# IO Daemon Contract

This document defines the shared contract for the JSTorrent IO daemon surface across:

- Node IO daemon
- Rust desktop IO daemon
- Android companion daemon

It is the normative source for:

- HTTP endpoint behavior
- `/io` and `/control` opcode meanings
- capability keys
- stream lifecycle semantics
- conformance case IDs

Machine-readable companions:

- [io-daemon-control-opcodes.json](../../contracts/io-daemon-control-opcodes.json)
- [io-daemon-conformance.json](../../contracts/io-daemon-conformance.json)

## Contract Metadata

Declared contract generation:

- `protocolVersion = 1`
- `behaviorVersion = 1`

These versions are not yet advertised by every implementation. The intended rollout is to expose them alongside the existing capability handshake and `/status` response.

Backward compatibility:

- version fields are additive and optional on the wire
- existing clients must continue to work when these fields are absent
- new clients must treat absent `protocolVersion` / `behaviorVersion` as a legacy daemon and fall back to capability-based behavior checks

Meaning:

- `protocolVersion` governs wire shape: endpoints, envelopes, field names, opcode numbers.
- `behaviorVersion` governs semantics: status codes, lifecycle rules, concurrency rules.

## Capability Model

Capabilities remain the runtime feature-gating mechanism. Versions define the contract generation; capabilities define which optional features are enabled within that generation.

Current capability keys:

- `health`
- `status`
- `ioWebSocket`
- `controlEvents`
- `rootsRead`
- `rootsWrite`
- `fileOps`
- `mediaCompleteFile206`
- `mediaBlocking206`

Rules:

- a daemon that advertises `mediaBlocking206: true` must satisfy the blocking stream behavior cases in the conformance catalog
- capabilities must not silently redefine the meaning of an existing contract case
- structural or semantic changes that affect core expectations should advance a contract version, not just flip a capability

## HTTP Surface

Baseline HTTP endpoints:

- `GET /health`
- `POST /status`
- `/status` may include optional `protocolVersion` and `behaviorVersion`
- `GET /network/interfaces`
- `GET /roots`
- `DELETE /roots/:key`
- `POST /hash/sha1` where supported
- `POST /files/ensure_dir`
- `POST /ops/truncate`
- `POST /ops/delete`
- `POST /ops/batch_delete`
- `GET /ops/stat`
- `GET /ops/exists`
- `GET /ops/list`
- `GET /ops/list_tree`
- `POST /ops/verify_chunks` where supported
- `GET|HEAD /stream/{token}`

Authentication:

- managed daemons require the daemon auth token for HTTP control/file operations
- companion implementations may additionally require extension headers depending on mode
- `/stream/{token}` is authorized by possession of the stream token

Baseline HTTP conformance cases:

- `health.ok_is_reported`
- `network.interfaces_are_reported`
- `roots.list_is_reported`
- `roots.delete_existing_root_succeeds`
- `hash.sha1_bytes_are_reported`
- `ops.exists_reports_presence`
- `ops.stat_reports_metadata`
- `ops.list_tree_reports_file_entries`
- `ops.list_reports_directory_entries`
- `files.ensure_dir_creates_directory`
- `ops.truncate_resizes_file`

Current implementation scope note:

- `GET /roots` is currently required in `node` and `android` for the managed daemon conformance matrix
- `DELETE /roots/:key` is currently required in `node` and `android`
- `GET /network/interfaces` is required in `node`, `rust`, and `android`
- `POST /hash/sha1` is currently required in `rust` and `android`; Node does not expose that endpoint in contract generation 1
- Rust standalone exposes `GET /roots`, but the managed native-host daemon path used by the default Rust conformance gate does not; `roots.*` therefore remains `N/A` for `rust` in contract generation 1

## File Operation Semantics

Presence and stat:

- `GET /ops/exists`
- returns `200` with JSON `{ "exists": boolean }`
- existing file paths must report `true`
- missing file paths must report `false`

- `GET /ops/stat`
- existing file paths must return `200` with JSON metadata containing:
  - `size`
  - `mtime`
  - `is_directory`
  - `is_file`
- missing file paths must return `404`

- `GET /ops/list_tree`
- returns `200` with a JSON array of file entries
- each entry must contain:
  - `path`
  - `size`
- returned paths are relative to the requested base path
- nonexistent directories return `200` with an empty array

- `GET /ops/list`
- returns `200` with a JSON array of entry names
- returns only the direct children of the requested directory

- `POST /files/ensure_dir`
- creates the requested directory path under the given root
- nested directories must be created as needed
- success is observed by effect and `200` status, not by a shared response body shape

- `POST /ops/truncate`
- truncates an existing file to the requested length
- success is observed by effect and `200` status, not by a shared response body shape

Current implementation scope note:

- `POST /ops/truncate` is currently required in `node` and `rust`
- Android has underlying truncate support in its file manager but does not expose `/ops/truncate` on the companion HTTP surface in contract generation 1

Single delete:

- `POST /ops/delete`
- if the target exists and is deleted, return `200`
- if the target path does not exist, return `404`
- if the root key is invalid, return root-auth/root-selection failure as implemented for that daemon

Batch delete:

- `POST /ops/batch_delete`
- returns `200` with a JSON array of failed entry names
- missing entries are ignored and must not be included as failures

This split is intentional:

- single delete is explicit and not-found is observable
- batch delete remains cleanup-friendly and idempotent

## Control Surface

The `/control` channel is the daemon control plane.

Current shared opcodes:

- `0xE0 ROOTS_CHANGED`
- `0xE1 EVENT`
- `0xE2 OPEN_FOLDER_PICKER`
- `0xE9 OPEN_FILE`
- `0xEA REVEAL_IN_FOLDER` or platform-equivalent open-folder action
- `0xEB POWER_HINT`
- `0xEC REGISTER_HTTP_STREAM`
- `0xED GET_CAPABILITIES`
- `GET_CAPABILITIES` may include optional `protocolVersion` and `behaviorVersion` alongside the existing capability payload
- `0xEE OPEN_HTTP_STREAM_SESSION`
- `0xEF WAIT_FOR_HTTP_STREAM_RANGE`
- `0xF0 CANCEL_HTTP_STREAM_RANGE_WAIT`
- `0xF1 CLOSE_HTTP_STREAM_SESSION`
- `0xF2 REVOKE_TORRENT_HTTP_STREAMS`

Opcode numbers and structural metadata are recorded in the JSON opcode manifest.

Implementation scope:

- the opcode manifest records `implementedIn` for opcodes that are not exposed by every daemon implementation
- desktop Rust currently implements the stream-control opcode subset plus `GET_CAPABILITIES`
- Node and Android expose the broader desktop-companion control surface including root notifications and local file/folder actions

Control conformance cases:

- `control.capabilities_are_reported`

## Stream Registration Model

`REGISTER_HTTP_STREAM` is torrent-owned, not path-owned.

Required stream identity:

- `streamToken`
- `torrentId`
- `fileIndex`
- `rootKey`
- `path`
- `fileSize`
- `mimeType`

Ownership:

- the registering control session owns the token
- the torrent lifecycle owns the token

Effects:

- closing the owning control session revokes its tokens
- removing the torrent revokes all tokens for that torrent
- stopping the torrent does not revoke the token

## HTTP Streaming Semantics

`GET /stream/{token}`:

- parses `Range`
- serves `206` for valid byte ranges
- serves from disk after wait/ready conditions are satisfied
- may block for incomplete but streamable ranges

`HEAD /stream/{token}`:

- must not open a byte-range wait session
- returns headers equivalent to a ranged `GET`

Range availability rules:

- complete ranges on disk serve immediately
- incomplete ranges may block only when the torrent is streamable
- incomplete non-streamable ranges fail early

Non-streamable examples:

- torrent stopped
- torrent inactive / not eligible to make progress
- torrent errored
- file skipped

Expected responses:

- stopped/inactive/errored/skipped incomplete range: `409`
- removed/revoked/missing stream session: `404`
- invalid range: `416`

## Lifecycle Rules

Torrent stopped:

- completed ranges remain serveable
- incomplete ranges fail early with `409`
- active waits abort
- tokens are not revoked

Torrent removed:

- tokens are revoked
- active waits abort
- new requests return `404`

HTTP disconnect:

- aborts only that request’s pending wait
- must not cancel other concurrent readers for the same token

## Concurrency Rules

One stream token may serve multiple concurrent readers.

Required behavior:

- concurrent readers may block independently
- canceling one reader must not cancel another
- stop/remove events fan out to all active readers correctly
- multi-chunk responses must re-enter wait logic per chunk window, not as a single whole-file wait

## Conformance Model

Conformance is tracked by stable case IDs from the conformance manifest.

Each implementation should satisfy the same required case IDs for a contract generation.

Recommended reporting shape:

- validate contract versions/capabilities first
- then run the required conformance cases for `node`, `rust`, and `android`
- report a per-case matrix rather than only raw test names

Current runner:

- `pnpm -C packages/engine run conformance:daemon`
- today this gates `node` and `rust`
- Android can be run explicitly with `pnpm -C packages/engine run conformance:daemon -- --implementations=android` when an emulator/device is available
- the Android runner installs the debug app/test APKs and executes the tagged instrumented classes directly through `adb shell am instrument`
- CI runs Android conformance in a dedicated emulator workflow rather than the default Node/Rust gate
- the first gate is limited to the shared Node/Rust cases currently implemented in managed mode; bootstrap `/status` remains Node/Android-only in the manifest for now
- Android currently participates in the daemon HTTP/media cases only; the `watch_video.*` daemon-backed engine cases remain Node/Rust-specific and are `N/A` for Android in the matrix

Initial case areas:

- status/capabilities
- file delete semantics
- stream blocking behavior
- stop/remove lifecycle behavior
- concurrent stream readers
- watch-video playback preparation
- daemon-backed root/default-root download flow
