# Web Seeds Design

## Goal

Add BEP 19 download support to `packages/engine` without distorting the existing tracker HTTP client or forcing the torrent engine to pretend HTTP sources are ordinary BitTorrent peers.

## Non-Goals

- No BEP 17 `httpseeds`
- No browser `fetch()` based implementation
- No attempt to unify all HTTP callers behind one public API today
- No upload/seeding over HTTP

## Design Summary

The design has three layers:

1. Shared HTTP core
2. Purpose-built `WebSeedHttpClient`
3. Torrent-side web-seed scheduler and source management

This keeps the current tracker client simple, gives web seeds the streaming and range controls they need, and preserves a path toward a broader plugin-facing HTTP stack later.

## Why Not Extend `MinimalHttpClient` Directly

`MinimalHttpClient` is control-plane HTTP.

Its current shape is appropriate for:

- Trackers
- Small request/response exchanges
- Fully buffered bodies

Web seeds are data-plane HTTP and need:

- Incremental body delivery
- Long-lived sockets
- `Range` requests
- Response header validation
- Redirect handling
- `Retry-After`
- `chunked` support
- Backpressure and cancellation tied to torrent state

If we add all of that directly into `MinimalHttpClient`, the tracker path becomes harder to reason about and plugin HTTP will inherit torrent-specific complexity.

## Module Layout

Recommended initial layout:

```text
packages/engine/src/http/
  http-transport.ts
  http-parser.ts
  http-types.ts
  url-utils.ts

packages/engine/src/tracker/
  tracker-http-client.ts

packages/engine/src/webseed/
  web-seed-types.ts
  web-seed-source.ts
  web-seed-http-client.ts
  web-seed-connection.ts
  web-seed-manager.ts
  web-seed-scheduler.ts
```

The existing tracker implementation can continue to use `MinimalHttpClient` initially.
The shared HTTP core should be introduced only where web seeds need it.

## Shared HTTP Core

The shared core is internal infrastructure, not a product-facing client API.

Responsibilities:

- Open and close TCP/TLS connections
- Serialize HTTP/1.1 requests
- Parse response status line and headers
- Provide incremental body delivery
- Support fixed-length and chunked bodies
- Support redirect processing
- Expose remote address and final URL
- Support cancellation via `AbortSignal`

Suggested types:

```typescript
export interface HttpRequest {
  method: 'GET'
  url: string
  headers?: Record<string, string>
  signal?: AbortSignal
}

export interface HttpResponseHead {
  statusCode: number
  statusMessage: string
  headers: Record<string, string>
  finalUrl: string
  remoteAddress?: string
}

export interface HttpBodyReader {
  read(): Promise<Uint8Array | null>
  cancel(reason?: string): void
}

export interface HttpTransportResponse {
  head: HttpResponseHead
  body: HttpBodyReader
}
```

Suggested transport entry point:

```typescript
export interface HttpTransport {
  request(request: HttpRequest): Promise<HttpTransportResponse>
}
```

This is a useful seam because:

- Web seeds can consume the streaming body directly
- Tracker and plugin clients can later wrap it with buffered convenience APIs
- The parser and socket code are shared without forcing shared caller behavior

## WebSeedHttpClient

`WebSeedHttpClient` is a purpose-built wrapper around the shared HTTP core.

Responsibilities:

- Make byte-range requests
- Validate range-related response headers
- Enforce `Accept-Encoding: identity`
- Expose response bytes as a stream
- Normalize retryable vs fatal errors
- Handle redirect and retry policy suitable for web seeds

Suggested interface:

```typescript
export interface WebSeedRangeRequest {
  url: string
  start: number
  endInclusive: number
  signal?: AbortSignal
}

export interface WebSeedRangeResponse {
  statusCode: number
  headers: Record<string, string>
  finalUrl: string
  remoteAddress?: string
  body: HttpBodyReader
}

export interface WebSeedHttpClient {
  requestRange(request: WebSeedRangeRequest): Promise<WebSeedRangeResponse>
}
```

Important point:

- This API is range-centric, not generic request-centric

That keeps torrent-specific validation local to the web-seed layer.

## Download Rate Limiting

Web-seed traffic must eventually respect the same download budgeting used for peer traffic.

Required end-state:

- Web-seed payload bytes consume the existing download token bucket / rate limit budget
- Download statistics continue to represent total download traffic across peers and web seeds
- Optional per-source accounting may be added for observability, but not as a separate primary limiter

Important rollout note:

- Initial web-seed integration does not have to block on rate-limit enforcement
- However, the torrent integration should be structured so shared rate limiting can be added cleanly without redesigning the web-seed scheduler

## Torrent Integration Model

Web seeds should not be implemented by teaching `PeerConnection` to speak HTTP.

Instead:

- `TorrentPieceRequester` continues to schedule BitTorrent peer requests
- `WebSeedScheduler` schedules larger contiguous byte spans for web-seed sources
- Received web-seed bytes are translated back into normal block insertions into active pieces

This preserves:

- Existing piece verification
- Existing disk write logic
- Existing endgame logic where possible
- Existing resume semantics

## Data Model Changes

Add torrent-level storage for parsed web seeds.

Suggested shapes:

```typescript
export interface ParsedTorrent {
  // existing fields
  urlSeeds?: string[]
}

export interface ParsedTorrentInput {
  // existing fields
  magnetUrlSeeds?: string[]
}
```

Torrent runtime state should distinguish:

- `metadataUrlSeeds`: from `.torrent` `url-list`
- `magnetUrlSeeds`: from magnet `ws`
- `activeWebSeeds`: normalized runtime source records after metadata is available

Suggested runtime source model:

```typescript
export interface WebSeedSource {
  id: string
  url: string
  kind: 'bep19'
  state: 'idle' | 'connecting' | 'active' | 'backoff' | 'disabled'
  failures: number
  nextRetryAt: number
  finalUrl?: string
  remoteAddress?: string
}
```

## Scheduler Model

The web-seed scheduler should optimize for contiguous bytes, not 16 KiB blocks.

Recommended initial strategy:

- Prefer pieces already active in `ActivePieceManager`
- Prefer sequential or streaming-hot regions
- Coalesce adjacent missing blocks into larger byte spans
- Bound each HTTP transfer by a configurable maximum request size
- Avoid assigning the same span to multiple web seeds except in explicit endgame cases

Initial defaults:

- One request in flight per `WebSeedConnection`
- Small number of concurrent web-seed connections per torrent
- Larger max request size than peer block requests

## How Bytes Enter The Torrent

The important rule is that web-seed bytes must enter the same integrity path as peer bytes.

Recommended flow:

1. Scheduler reserves missing blocks in an active piece under a web-seed source ID
2. Scheduler coalesces those reserved blocks into one byte-range request
3. `WebSeedConnection` streams bytes back
4. Streamed bytes are split back into block boundaries
5. Torrent inserts those blocks into `ActivePieceManager`
6. Piece completion triggers the normal hash verification and write path

To make this clean, the torrent should gain a source-agnostic block insertion method.

Suggested abstraction:

```typescript
export interface BlockSourceRef {
  id: string
  kind: 'peer' | 'webseed'
}
```

Then the common insertion path can validate that:

- The block belongs to an active piece
- The source holds the reservation
- The byte count matches the reserved block/span

## Reservation Model

Today active-piece bookkeeping is peer-centric.
We should generalize that to source-centric bookkeeping where the source can be either:

- A BitTorrent peer ID
- A web-seed source ID

This is the minimum change needed to reuse the existing verification pipeline without pretending a web seed is a peer-wire endpoint.

## Redirect Policy

Initial redirect policy:

- Allow redirects for GET
- Cap redirect depth
- Preserve retry metadata on the resulting source record
- Record `finalUrl` for observability
- Keep redirect handling inside `WebSeedHttpClient`

Deferred capability:

- Per-file redirects for multi-file torrents

The design should leave room for per-file redirect maps later, but initial implementation can treat redirects at the request/source level.

## Retry and Backoff Policy

Web-seed retry policy should be independent of tracker retry policy.

Recommended rules:

- Honor `Retry-After`
- Exponential backoff for transport and `5xx` failures
- Fast disable on repeated protocol violations
- Separate corruption handling from transport retry

Possible categories:

- `transport_error`
- `http_retryable`
- `http_fatal`
- `protocol_violation`
- `hash_failure`

## Source Reputation

The scheduler should track lightweight source reputation.

Useful signals:

- Recent throughput
- Consecutive failures
- Hash failures
- Whether the source supports keep-alive reliably
- Redirect churn
- Future interaction with shared download rate limiting

Initial implementation can keep this simple:

- Penalize recent failures
- Prefer sources with successful recent transfers

## Security Constraints

The first implementation should include basic protections:

- Limit redirect depth
- Disable compressed responses
- Validate response framing strictly
- Avoid permissive host rewriting
- Keep SSRF mitigation requirements visible in the interface even if full policy comes later

This matters because the same HTTP core may later be used by plugins.

## Relationship To Future Plugin HTTP

This design keeps that path open.

Future direction:

- `PluginHttpClient` can wrap the shared HTTP core
- Tracker HTTP can eventually wrap the same core with buffered helpers
- Web seeds remain a separate domain-specific wrapper because they are range-centric and torrent-aware

That means we are sharing infrastructure, not prematurely merging all behaviors.

## Rollout Plan

### Phase 1: Metadata And Docs

- Parse `.torrent` `url-list`
- Persist magnet `ws`
- Add torrent runtime fields for web seeds
- Land design docs

### Phase 2: Shared HTTP Core

- Implement streaming response parser
- Support fixed-length and chunked bodies
- Support TLS, redirects, cancellation, and remote address reporting

### Phase 3: WebSeed Client

- Implement `requestRange()`
- Validate `206` and `Content-Range`
- Add retry and redirect policy

### Phase 4: Torrent Integration

- Add `WebSeedManager`
- Generalize active-piece reservations from peer-centric to source-centric
- Add source-agnostic block insertion path
- Keep the scheduling path ready for shared download-bandwidth enforcement

### Phase 5: Scheduling And Limits

- Add contiguous-span scheduler
- Add per-torrent and global web-seed connection limits
- Add telemetry and debug state
- Wire web-seed payload bytes into the existing download rate limiter / token bucket

### Phase 6: Hardening

- Corruption penalties
- Better redirect behavior
- Better streaming/video bias
- More aggressive coalescing heuristics

## Test Plan

We should add targeted tests, not just one end-to-end case.

Parser tests:

- `.torrent` with string `url-list`
- `.torrent` with list `url-list`
- Magnet with multiple `ws`

HTTP transport tests:

- Fixed `Content-Length`
- `chunked` body
- redirect chain
- `Retry-After`
- abort mid-stream

Web-seed integration tests:

- Single-file range fetch
- Multi-file range fetch crossing file boundary
- Sequential streaming bias
- Resume after partial download
- Corrupt source causing hash failure

## Open Questions

- Whether per-file redirect tracking is needed in the first milestone
- Whether web-seed reservations should reuse existing request timestamp tracking or maintain separate timing
- Whether initial scheduling should pull only from already-active pieces or also activate new pieces directly
- Whether rate limiting should pause socket reads, scheduler dispatch, or both in the first enforcement pass

## Current Recommendation

Build a shared internal HTTP transport, keep web seeds on a purpose-built client, and integrate them into the torrent as source-aware ranged downloaders rather than fake peers.
