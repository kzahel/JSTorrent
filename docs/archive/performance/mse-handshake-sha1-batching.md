# MSE Handshake SHA1 Batching

## Problem

Each MSE (Message Stream Encryption) connection handshake requires 4-5 SHA1 calls that depend on the DH shared secret. Currently these are sequential HTTP requests to the daemon, adding latency to every peer connection.

**Current cost per connection:** 4-5 SHA1 HTTP round trips

## Background

The SHA1 calls during MSE handshake:

| Hash | Formula | When |
|------|---------|------|
| keyA | `SHA1('keyA' + S + infoHash)` | After DH exchange |
| keyB | `SHA1('keyB' + S + infoHash)` | After DH exchange |
| req1 | `SHA1('req1' + S)` | For sync pattern |
| req2 | `SHA1('req2' + infoHash)` | For torrent ID (initiator only) |
| req3 | `SHA1('req3' + S)` | For torrent ID |

These cannot be precomputed because `S` (shared secret) is unique per connection, derived from Diffie-Hellman key exchange.

Note: For incoming connections, `req2` is looked up from a precomputed map rather than computed per-connection.

## Solution

Batch all SHA1 calls into a single HTTP request using the existing `sha1Batch` endpoint.

**Target cost per connection:** 1 SHA1 HTTP round trip

## Implementation

### Initiator (outgoing connection)

In `mse-handshake.ts`, after computing `sharedSecret`:

```typescript
// Current: 5 sequential calls
const keyBytes = await deriveEncryptionKeyBytes(sharedSecret, infoHash, true, sha1)  // 2 calls
const req1Hash = await computeReq1Hash(sharedSecret, sha1)                           // 1 call
const req2Xor3 = await computeReq2Xor3(infoHash, sharedSecret, sha1)                 // 2 calls

// Proposed: 1 batch call
const [keyA, keyB, req1, req2, req3] = await sha1Batch([
  concat(encode('keyA'), sharedSecret, infoHash),
  concat(encode('keyB'), sharedSecret, infoHash),
  concat(encode('req1'), sharedSecret),
  concat(encode('req2'), infoHash),
  concat(encode('req3'), sharedSecret),
])

const encryptKey = keyA  // initiator uses keyA for encrypt
const decryptKey = keyB
const req2Xor3 = xor(req2, req3)
```

### Responder (incoming connection)

After DH exchange and recovering infoHash from the map:

```typescript
// Current: 3 sequential calls
const req1Hash = await computeReq1Hash(sharedSecret, sha1)                    // 1 call
const infoHash = await recoverInfoHashWithMap(xorValue, sharedSecret, ...)    // 1 call (req3)
const keys = await deriveEncryptionKeys(sharedSecret, infoHash, false, sha1)  // 2 calls

// Proposed: batch where possible
// Note: req3 must be computed first to recover infoHash, then keyA/keyB
// So this is 2 batches at best: [req1, req3] then [keyA, keyB]
// Or restructure to do req3 alone, then batch [req1, keyA, keyB]
```

The responder flow is trickier because we need `req3` to recover the infoHash before we can compute `keyA`/`keyB`. Options:

1. **Two batches:** `[req1, req3]` then `[keyA, keyB]` = 2 HTTP calls (down from 4)
2. **Speculative:** Compute `req3` alone, recover infoHash, then batch `[req1, keyA, keyB]` = 2 HTTP calls

### Changes Required

1. **`mse-handshake.ts`**
   - Add `sha1Batch` to `MseHandshakeOptions` interface
   - Refactor `processPe2` (initiator) to batch all 5 hashes
   - Refactor `processPe3AfterSync` (responder) to batch where possible

2. **`mse-socket.ts`**
   - Pass `sha1Batch` through options

3. **`bt-engine.ts` / `connection-manager.ts`**
   - Pass `hasher.sha1Batch` when creating MseSocket

### Fallback

If `sha1Batch` is not available (e.g., browser with SubtleCrypto), fall back to sequential calls:

```typescript
if (sha1Batch) {
  const [keyA, keyB, req1, req2, req3] = await sha1Batch([...])
} else {
  // existing sequential code
}
```

## Impact

| Scenario | Before | After |
|----------|--------|-------|
| Outgoing connection | 5 HTTP calls | 1 HTTP call |
| Incoming connection | 4 HTTP calls | 2 HTTP calls |
| 20 peer connections | ~100 HTTP calls | ~20-30 HTTP calls |

## Status

- [x] Implementation
- [x] Tests
- [ ] Benchmarking
