# BEP 19 URL Seeds

## Scope

This document captures the parts of BEP 19 that matter for `packages/engine`.
It is intentionally implementation-oriented.

Status for JSTorrent:

- In scope: `url-list` from `.torrent` files
- In scope: `ws` magnet parameter as a source of BEP 19 URLs
- Out of scope: BEP 17 `httpseeds`

## Why We Want It

URL seeds let a torrent fetch payload bytes over HTTP or HTTPS in addition to the BitTorrent swarm.
For JSTorrent this is useful as:

- A fallback source when the peer swarm is weak
- A bootstrap source for rare torrents
- A source that can coexist with piece verification and resume logic already in the engine

## Metadata Sources

JSTorrent should accept BEP 19 URLs from two places:

1. `.torrent` top-level `url-list`
2. Magnet `ws` parameters

Important distinction:

- `.torrent` `url-list` is authoritative torrent metadata
- Magnet `ws` is only an external hint until metadata arrives

## `url-list` Encoding

`url-list` may be:

- A single string URL
- A list of string URLs

Implementation requirements:

- Ignore empty entries
- Deduplicate exact duplicate URLs
- Preserve ordering for scheduler preference
- Support both `http:` and `https:`

## Single-File Semantics

For a single-file torrent, the URL seed points to the file contents.

Examples:

- Torrent name: `movie.mkv`
- URL seed: `https://cdn.example.com/movie.mkv`

The client issues HTTP `GET` requests with a `Range` header matching the desired byte span in the file.

Example:

```http
GET /movie.mkv HTTP/1.1
Host: cdn.example.com
Range: bytes=1048576-1114111
Connection: keep-alive
Accept-Encoding: identity
```

## Multi-File Semantics

For a multi-file torrent, the URL seed acts as the root directory for the torrent payload.

Example torrent layout:

```text
foo/
  a.bin
  dir/b.bin
```

Example root URL:

```text
https://cdn.example.com/foo/
```

Requests are made against file paths underneath the root. Piece spans may cross file boundaries, so one piece request may turn into multiple HTTP range requests.

Examples:

- `https://cdn.example.com/foo/a.bin`
- `https://cdn.example.com/foo/dir/b.bin`

Implementation notes:

- Treat the URL as a directory root for multi-file torrents
- Normalize missing trailing slash on multi-file roots
- Preserve percent-encoding carefully
- Do not assume browser URL APIs are safe for raw torrent paths

## Range Requests

Range requests are the core transport mechanism.

Requirements:

- Send `Range: bytes=start-end`
- Accept `206 Partial Content`
- Accept `200 OK` only when the returned body exactly matches the requested full object or the requested range can be validated safely
- Validate `Content-Range` whenever present
- Reject responses whose byte framing does not match the requested span

The engine should treat HTTP framing mismatches as transport errors, not as piece corruption.

## Redirects

Redirects matter in practice and should be part of the initial design.

Required behavior:

- Support standard HTTP redirects for GET requests
- Track the final URL for telemetry/debugging
- Limit redirect depth
- Apply the same redirect policy to HTTP and HTTPS

Security and correctness constraints:

- Do not allow unlimited redirect chains
- Preserve per-source retry state separately from redirected targets when practical
- Do not silently downgrade safety assumptions during redirects

For multi-file torrents, per-file redirects can exist in the real world. JSTorrent does not need that on day one if the abstraction allows it, but the design should not rule it out.

## Keep-Alive and Pipelining

HTTP keep-alive is important for performance.

The client should:

- Reuse a socket for sequential range requests when the server permits it
- Avoid browser-style fetch semantics that hide connection lifecycle
- Prefer large contiguous reads rather than one HTTP request per 16 KiB block

We do not need full HTTP/1.1 request pipelining in the first implementation.

Recommended initial policy:

- One active request per web-seed connection
- Reuse the connection across many sequential range requests
- Permit multiple concurrent web-seed connections globally, but keep them bounded

This keeps the scheduler simpler while still capturing most of the performance win from contiguous reads.

## Content Encoding

The client should request identity encoding.

Requirements:

- Send `Accept-Encoding: identity`
- Reject compressed response bodies
- Support `chunked` transfer encoding at the HTTP framing layer

Even with `Range`, some servers may respond with chunked transfer framing. The transport layer must support this.

## Error Handling

Transport-level retry should be separate from piece hash verification.

HTTP errors:

- `404`: likely missing file or bad seed URL
- `416`: bad range or server behavior mismatch
- `429` / `503`: retryable, especially if `Retry-After` exists
- `5xx`: generally retryable with backoff

Required behaviors:

- Honor `Retry-After` when present
- Back off failed sources
- Disable or de-prioritize persistently broken sources
- Distinguish "source is bad" from "piece hash mismatch"

## Integrity Model

URL seeds are not trusted.

All payload data from web seeds must still flow through normal piece hash verification.

Implications:

- HTTP success does not imply correctness
- Corrupt data should affect source reputation
- Resume data should continue to be keyed off verified pieces, not downloaded byte counts

## Magnet Integration

Magnet `ws` support is useful, but there is an important lifecycle constraint:

- Until the info dictionary arrives, the client does not know file layout
- Therefore it cannot issue BEP 19 file/path based requests yet

So JSTorrent should:

- Parse and store `ws` URLs at add-torrent time
- Activate them only after metadata has been verified and file layout is known

## Non-Goals

The following are explicitly out of scope for the first implementation:

- BEP 17 `httpseeds`
- Browser `fetch()` as the core transport
- HTTP/2 or HTTP/3
- Uploading or seeding over HTTP
- Full parity with libtorrent's per-file redirect and partial-file availability logic

## Design Consequences For JSTorrent

BEP 19 pushes us toward:

- A streaming HTTP transport with explicit socket lifecycle
- Range-aware request planning
- Piece-to-file mapping for multi-file torrents
- A purpose-built web-seed scheduler that prefers contiguous reads
- Reuse of existing piece verification and disk write paths
- Eventual integration with the engine's existing download rate limiting, even if the first milestone lands before that enforcement is wired up

## References

- BEP 19: <https://www.bittorrent.org/beps/bep_0019.html>
- Libtorrent `torrent_info.cpp`: parsing `url-list`
- Libtorrent `web_peer_connection.cpp`: practical range-request behavior
