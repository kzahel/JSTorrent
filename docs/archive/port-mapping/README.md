# Port Mapping: NAT-PMP + PCP + UPnP

Add NAT-PMP (RFC 6886) and PCP (RFC 6887) support alongside the existing UPnP
IGD implementation. Unified `PortMappingManager` tries PCP → NAT-PMP → UPnP.

## Why JSTorrent First

- Working UDP stack across all runtimes (Node, QuickJS/Android, daemon/browser)
- Working UPnP stack proves the architecture
- Real users on Apple routers and OpenWrt where UPnP is unavailable
- web-server will port from here — building it tested and proven first
- NAT-PMP/PCP are pure binary UDP — trivial once you have `IUdpSocket`

## Phases

### [Phase 1: Refactor](./phase-1-refactor.md) — No New Behavior

Rename `upnp/` → `port-mapping/`, extract `PortMappingProvider` interface, evolve
`UPnPManager` → `PortMappingManager`. All existing behavior preserved. UPnP still
the only active protocol.

### Phase 2: NAT-PMP Client + Tests

Implement `nat-pmp-client.ts` — binary encode/decode, retry with backoff, external
address query, port mapping requests. Unit tests with mock `IUdpSocket`.

### Phase 3: PCP Client + Tests

Implement `pcp-client.ts` — MAP opcode, nonce handling, IPv4-mapped-IPv6 encoding,
response validation. Unit tests with mock socket.

### Phase 4: Gateway Detection

Platform-specific default gateway discovery:
- Node: parse `netstat -rn` (macOS) / `ip route` (Linux)
- Native/Android: new `__jstorrent_get_default_gateway` JNI binding
- Daemon: new endpoint on io-daemon or Android companion

### Phase 5: Unified Fallback Manager

Wire PCP → NAT-PMP → UPnP into `PortMappingManager.discover()`. Lifetime-aware
renewal (use server-granted lifetime, not hardcoded 30 min). Update `bt-engine.ts`
to pass `getDefaultGateway`.

### Phase 6: Integration Testing

Mock NAT-PMP/PCP server (~50 lines, in-process UDP) for CI. Manual smoke test
against real Apple/OpenWrt router before shipping.

---

## Protocol Summary

| Protocol | Version | Port | Transport | Request/Response Size |
|----------|---------|------|-----------|-----------------------|
| NAT-PMP  | 0       | 5351 | UDP unicast to gateway | 12B req / 16B resp |
| PCP      | 2       | 5351 | UDP unicast to gateway | 60B req / 60B resp |
| UPnP IGD | —       | 1900 | UDP multicast (SSDP) + HTTP/SOAP | Variable |

### NAT-PMP Packet Format (RFC 6886)

**External Address Request** (2 bytes):
```
Byte 0: Version = 0
Byte 1: Opcode = 0 (external address)
```

**External Address Response** (12 bytes):
```
Byte  0:    Version = 0
Byte  1:    Opcode = 128 (0 | 0x80)
Bytes 2-3:  Result code (uint16 BE)
Bytes 4-7:  Seconds since epoch (uint32 BE)
Bytes 8-11: External IPv4 address (4 bytes)
```

**Port Mapping Request** (12 bytes):
```
Byte  0:    Version = 0
Byte  1:    Opcode (1=UDP, 2=TCP)
Bytes 2-3:  Reserved (0x0000)
Bytes 4-5:  Internal port (uint16 BE)
Bytes 6-7:  External port (uint16 BE, 0=let gateway choose)
Bytes 8-11: Lifetime in seconds (uint32 BE, 0=delete)
```

**Port Mapping Response** (16 bytes):
```
Byte  0:    Version = 0
Byte  1:    Opcode | 0x80 (129=UDP, 130=TCP)
Bytes 2-3:  Result code (uint16 BE)
Bytes 4-7:  Seconds since epoch (uint32 BE)
Bytes 8-9:  Internal port (uint16 BE)
Bytes 10-11: External port (uint16 BE)
Bytes 12-15: Lifetime (uint32 BE)
```

**NAT-PMP Result Codes:**
- 0: Success
- 1: Unsupported version
- 2: Not authorized (e.g. port mapping disabled on router)
- 3: Network failure
- 4: Out of resources
- 5: Unsupported opcode

**Retry:** 250ms initial, double each retry, max 9 attempts (RFC 6886 §3.1).

### PCP Packet Format (RFC 6887)

**Request Header** (24 bytes):
```
Byte  0:     Version = 2
Byte  1:     Opcode (1=MAP)
Bytes 2-3:   Reserved
Bytes 4-7:   Lifetime (uint32 BE)
Bytes 8-23:  Client IP address (IPv6 or IPv4-mapped-IPv6, 16 bytes)
```

**MAP Opcode Extension** (follows header, 36 bytes):
```
Bytes 24-35: Nonce (12 random bytes, used for matching)
Byte  36:    Protocol (6=TCP, 17=UDP, 0=all)
Bytes 37-39: Reserved
Bytes 40-41: Internal port (uint16 BE)
Bytes 42-43: External port (uint16 BE, 0=let gateway choose)
Bytes 44-59: Suggested external IP (IPv6 or IPv4-mapped, 16 bytes)
```

**Response Header** (24 bytes):
```
Byte  0:     Version = 2
Byte  1:     Opcode | 0x80 (R bit set)
Byte  2:     Reserved
Byte  3:     Result code (uint8)
Bytes 4-7:   Lifetime (uint32 BE)
Bytes 8-11:  Gateway epoch seconds (uint32 BE)
Bytes 12-23: Reserved
```

**MAP Response Extension** (follows header, 36 bytes):
```
Bytes 24-35: Nonce (12 bytes, echoed back)
Byte  36:    Protocol
Bytes 37-39: Reserved
Bytes 40-41: Internal port (uint16 BE)
Bytes 42-43: External port (uint16 BE, assigned by gateway)
Bytes 44-59: External IP address (IPv6 or IPv4-mapped, 16 bytes)
```

**PCP Result Codes:**
- 0: Success
- 1: Unsupported version (triggers NAT-PMP fallback)
- 2: Not authorized
- 3: Malformed request
- 4: Unsupported opcode
- 5-13: Various (see RFC 6887 §7.4)

**IPv4-Mapped-IPv6:** PCP always uses 16-byte addresses. IPv4 addresses encoded as
`::ffff:a.b.c.d` (bytes 0-9 zero, bytes 10-11 `0xFFFF`, bytes 12-15 IPv4 address).

**Nonce:** 12 random bytes generated per mapping. Server echoes back in response.
Used for renewal and deletion of existing mappings.

**Retry:** Initial 1s, double each retry (with ±10% jitter), max 3 retries (crab_nat default).
RFC allows more but 3 is practical.

### PCP ↔ NAT-PMP Fallback

Both share port 5351. Try PCP first. If the router only supports NAT-PMP, it
replies with result code 1 (unsupported version) containing version 0. On seeing
this, retry with NAT-PMP. This is one round-trip of overhead.

libtorrent: starts PCP, on `pcp_unsupp_version`, sets `m_version = version_natpmp`
and resends. crab_nat: tries PCP, if `UnsupportedVersion(NatPmp)`, silently falls
back. Both treat other PCP errors as fatal (don't fall back).

---

## Current Implementation (pre-refactor)

### Files (530 lines total)

```
packages/engine/src/upnp/
├── index.ts              5 lines   Exports
├── upnp-manager.ts     190 lines   Orchestrator: discover → map → renew → cleanup
├── ssdp-client.ts      116 lines   SSDP M-SEARCH multicast discovery
└── gateway-device.ts   219 lines   UPnP SOAP control (add/delete/list mappings)
```

### Key Interfaces Used

- `IUdpSocket` (`interfaces/socket.ts:97-123`): `send()`, `onMessage()`, `close()`, `joinMulticast()`, `leaveMulticast()`
- `ISocketFactory.createUdpSocket()`: factory method
- `MinimalHttpClient` (`utils/minimal-http-client.ts`): TCP HTTP for SOAP calls
- `NetworkInterface`: `{name, address, prefixLength}` for subnet matching

### Integration Points (17 files reference UPnP)

**Engine core:**
- `core/bt-engine.ts:21` — imports `UPnPManager`, `NetworkInterface`
- `core/bt-engine.ts:49` — defines `UPnPStatus` type
- `core/bt-engine.ts:253` — `private upnpManager?: UPnPManager`
- `core/bt-engine.ts:554` — startup: `if (config.upnpEnabled) enableUPnP()`
- `core/bt-engine.ts:1137` — suspend: `await disableUPnP()`
- `core/bt-engine.ts:1333` — config change listener
- `core/bt-engine.ts:1841-1891` — `enableUPnP()` / `disableUPnP()` methods

**Config:**
- `config/config-schema.ts:21` — `UPnPStatus` type
- `config/config-schema.ts:376` — `upnpEnabled` setting (default `true`)
- `config/config-schema.ts:665` — `upnpStatus` runtime value
- `config/config-hub.ts:21,269` — hub accessor
- `config/base-config-hub.ts:107,154` — config values
- `config/index.ts:32` — re-export

**Adapters:**
- `adapters/daemon/daemon-connection.ts:1` — imports `NetworkInterface`
- `presets/native.ts:20` — imports `NetworkInterface`
- `adapters/native/controller.ts:1073` — `__jstorrent_query_upnp_status`

**Main exports:**
- `index.ts:114-115` — re-exports `UPnPManager`, `SSDPClient`, `GatewayDevice`, types

**Client/Extension (3 files):**
- `packages/client/.../SettingsOverlay.tsx` — UPnP status display + toggle
- `packages/client/.../ConfigContext.tsx` — watches `upnpEnabled`
- `packages/client/.../daemon-engine-manager.ts` — passes `getNetworkInterfaces`

**Android (6 files):**
- `EngineModels.kt`, `ConfigBridge.kt`, `AndroidConfigHub.kt`
- `JSTorrentApplication.kt`, `SettingsViewModel.kt`, `NetworkSettingsScreen.kt`

---

## Reference Implementations

| Repo | Path | Key Files | Notes |
|------|------|-----------|-------|
| libtorrent | `~/code/libtorrent/` | `src/natpmp.cpp` (929L), `include/libtorrent/natpmp.hpp`, `include/libtorrent/portmap.hpp` | **Primary ref.** Production BitTorrent client, PCP+NAT-PMP with version fallback |
| libnatpmp | `~/code/references/libnatpmp/` | `natpmp.c` (403L), `natpmp.h` (222L) | Minimal NAT-PMP client. Best for understanding the binary protocol |
| miniupnp | `~/code/references/miniupnp/` | `miniupnpd/pcpserver.c` (1758L), `miniupnpd/natpmp.c` (491L) | **Server side** — shows what routers validate |
| crab_nat | `~/code/references/crab_nat/` | `src/natpmp.rs` (414L), `src/pcp.rs` (1187L), `src/lib.rs` (448L) | Async Rust, both protocols. Cleanest PCP MAP impl |
| go-nat-pmp | `~/code/references/go-nat-pmp/` | `natpmp.go` (157L) | Quickest read — whole protocol in 157 lines |

### Reading Order

1. `go-nat-pmp/natpmp.go` — 5 min, grasp the packet format
2. `libtorrent/src/natpmp.cpp` — PCP→NAT-PMP fallback, retry logic, IPv4-mapped-IPv6
3. `crab_nat/src/pcp.rs` — MAP opcode details, nonce, response validation
4. `miniupnp/miniupnpd/pcpserver.c` — what routers actually check

---

## Design Decisions

### Naming

- Config key stays `upnpEnabled` (persisted in user storage, backward compat).
  Internally controls the broader port mapping system.
- Status type stays compatible: `'disabled' | 'discovering' | 'mapped' | 'unavailable' | 'failed'`
- New: `activeProtocol: 'pcp' | 'nat-pmp' | 'upnp' | null` exposed to UI

### Lease / Renewal

Current UPnP: hardcoded 1h lease, renew every 30 min.
NAT-PMP/PCP: server may grant different lifetime than requested.
New approach: request 2h (recommended by crab_nat / RFC), renew at 75% of
server-granted lifetime (libtorrent pattern), minimum 5 min renewal interval.

### Description String

Currently hardcoded `'JSTorrent'` in `addMapping()`. Make it a constructor param
so web-server can pass `'200 OK Web Server'`.

### `NetworkInterface` Ownership

Currently defined in `upnp-manager.ts`. Move to `interfaces/` (it's a general
networking concept, not UPnP-specific). Used by `daemon-connection.ts`,
`presets/native.ts`, and soon by gateway detection.

### Shared Code for web-server

`port-mapping/` will depend only on:
- `IUdpSocket` / `ISocketFactory` (interface, already shared)
- `MinimalHttpClient` (util, already copyable)
- `NetworkInterface` (will be in interfaces/)

Copy story: copy `port-mapping/` + `utils/minimal-http-client.ts`, change description
string, wire up `getDefaultGateway` per platform.
