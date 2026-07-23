# MSE SHA1 Optimization Plan

## Problem Statement

Incoming MSE (Message Stream Encryption) connections require identifying which torrent a peer wants to connect to. The current `recoverInfoHash()` function computes `SHA1('req2' + infoHash)` for **every known torrent** until it finds a match. With N torrents, this means N SHA1 HTTP round trips per incoming connection.

**Current cost:** O(N) SHA1 HTTP calls per incoming MSE handshake
**Target cost:** O(1) lookup + 1 SHA1 call per incoming MSE handshake

## Solution Overview

Two complementary optimizations:

1. **Batch SHA1 endpoint** - Reduce HTTP overhead for multiple hash operations
2. **Precomputed req2 cache** - Eliminate repeated computation entirely for incoming connections

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                           BtEngine                                   │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  torrents: Torrent[]                                         │   │
│  │    └─ each has _req2Hash: Uint8Array (precomputed)          │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                       │
│                              ▼                                       │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  On incoming connection:                                     │   │
│  │    req2Map = buildReq2Map(torrents)  // O(N) map build      │   │
│  │    pass to MseSocket                                         │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                       │
│                              ▼                                       │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  MseHandshake.recoverInfoHash()                              │   │
│  │    1. SHA1('req3' + sharedSecret)  // 1 HTTP call           │   │
│  │    2. XOR with received value                                │   │
│  │    3. Map lookup  // O(1)                                    │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘

Precomputation happens in initializeTorrentMetadata():
  - For .torrent files: immediate
  - For magnet links: when metadata arrives
  - Uses batch SHA1 if multiple torrents loaded at startup
```

---

## Phase 1: Batch SHA1 Endpoint (Rust)

**Goal:** Add `POST /hash/sha1/batch` to io-daemon

### Files to Modify
- `desktop/io-daemon/src/hashing.rs`

### Wire Format

**Request:** Length-prefixed binary
```
┌──────────────┬──────────────┬───────────┬──────────────┬───────────┬─────┐
│ count (u32)  │ len1 (u32)   │ data1     │ len2 (u32)   │ data2     │ ... │
│ little-end   │ little-end   │ [len1 B]  │ little-end   │ [len2 B]  │     │
└──────────────┴──────────────┴───────────┴──────────────┴───────────┴─────┘
```

**Response:** Concatenated 20-byte hashes
```
┌──────────────┬──────────────┬─────┐
│ hash1 (20B)  │ hash2 (20B)  │ ... │
└──────────────┴──────────────┴─────┘
```

### Implementation

```rust
// In hashing.rs

async fn hash_sha1_batch(body: Bytes) -> Result<impl Reply, Rejection> {
    if body.len() < 4 {
        return Err(warp::reject::custom(BadRequest("Body too short")));
    }

    let count = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;

    // Sanity limit: max 10,000 items
    if count > 10_000 {
        return Err(warp::reject::custom(BadRequest("Too many items")));
    }

    let mut offset = 4;
    let mut results = Vec::with_capacity(count * 20);

    for _ in 0..count {
        if offset + 4 > body.len() {
            return Err(warp::reject::custom(BadRequest("Truncated input")));
        }

        let len = u32::from_le_bytes(body[offset..offset+4].try_into().unwrap()) as usize;
        offset += 4;

        if offset + len > body.len() {
            return Err(warp::reject::custom(BadRequest("Truncated input")));
        }

        let data = &body[offset..offset+len];
        offset += len;

        let hash = Sha1::digest(data);
        results.extend_from_slice(&hash);
    }

    Ok(Response::builder()
        .header("Content-Type", "application/octet-stream")
        .body(results))
}

// Add to routes()
let batch_sha1 = warp::path!("hash" / "sha1" / "batch")
    .and(warp::post())
    .and(warp::body::bytes())
    .and_then(hash_sha1_batch);
```

### Testing

```bash
cd desktop/io-daemon
cargo build
cargo test

# Manual test with curl
echo -n $'\x02\x00\x00\x00\x05\x00\x00\x00hello\x05\x00\x00\x00world' | \
  curl -X POST --data-binary @- http://localhost:PORT/hash/sha1/batch | xxd
```

### Acceptance Criteria
- [ ] Endpoint accepts length-prefixed binary format
- [ ] Returns concatenated 20-byte hashes
- [ ] Returns 400 for malformed input
- [ ] Returns 400 for count > 10,000
- [ ] Unit tests pass

---

## Phase 2: Batch SHA1 Endpoint (Kotlin)

**Goal:** Add `POST /hash/sha1/batch` to companion-server

### Files to Modify
- `android/companion-server/src/main/java/com/jstorrent/companion/server/NettyHttpServer.kt`

### Implementation

```kotlin
// In NettyHttpServer.kt

private fun handleHashSha1Batch(request: FullHttpRequest): FullHttpResponse {
    val body = request.content()

    if (body.readableBytes() < 4) {
        return errorResponse(HttpResponseStatus.BAD_REQUEST, "Body too short")
    }

    val count = body.readIntLE()

    if (count > 10_000) {
        return errorResponse(HttpResponseStatus.BAD_REQUEST, "Too many items")
    }

    val results = ByteArray(count * 20)
    val md = MessageDigest.getInstance("SHA-1")

    for (i in 0 until count) {
        if (body.readableBytes() < 4) {
            return errorResponse(HttpResponseStatus.BAD_REQUEST, "Truncated input")
        }

        val len = body.readIntLE()

        if (body.readableBytes() < len) {
            return errorResponse(HttpResponseStatus.BAD_REQUEST, "Truncated input")
        }

        val data = ByteArray(len)
        body.readBytes(data)

        md.reset()
        val hash = md.digest(data)
        System.arraycopy(hash, 0, results, i * 20, 20)
    }

    return binaryResponse(results)
}

// In route matching:
"/hash/sha1/batch" -> handleHashSha1Batch(request)
```

### Testing

```bash
cd android
./gradlew :companion-server:compileDebugKotlin
./gradlew :companion-server:testDebugUnitTest
```

### Acceptance Criteria
- [x] Same wire format as Rust implementation
- [x] Same error handling (400 for malformed, count limit)
- [x] Unit tests pass

---

## Phase 3: TypeScript Client

**Goal:** Add `sha1Batch()` to DaemonHasher and IHasher interface

### Files to Modify
- `packages/engine/src/interfaces/hasher.ts`
- `packages/engine/src/adapters/daemon/daemon-hasher.ts`

### IHasher Interface Update

```typescript
// In hasher.ts
export interface IHasher {
  sha1(data: Uint8Array): Promise<Uint8Array>

  /**
   * Batch SHA1 computation. Optional - falls back to sequential if not implemented.
   * @param inputs - Array of data to hash
   * @returns Array of 20-byte hashes in same order
   */
  sha1Batch?(inputs: Uint8Array[]): Promise<Uint8Array[]>
}
```

### DaemonHasher Implementation

```typescript
// In daemon-hasher.ts

async sha1Batch(inputs: Uint8Array[]): Promise<Uint8Array[]> {
  if (inputs.length === 0) return []
  if (inputs.length === 1) return [await this.sha1(inputs[0])]

  // Encode length-prefixed format
  let totalSize = 4 // count
  for (const input of inputs) {
    totalSize += 4 + input.length // len + data
  }

  const buffer = new ArrayBuffer(totalSize)
  const view = new DataView(buffer)
  const bytes = new Uint8Array(buffer)

  view.setUint32(0, inputs.length, true) // little-endian count
  let offset = 4

  for (const input of inputs) {
    view.setUint32(offset, input.length, true)
    offset += 4
    bytes.set(input, offset)
    offset += input.length
  }

  const response = await this.fetch(`${this.baseUrl}/hash/sha1/batch`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/octet-stream' },
    body: bytes,
  })

  if (!response.ok) {
    throw new Error(`Batch SHA1 failed: ${response.status}`)
  }

  const resultBuffer = await response.arrayBuffer()
  const resultBytes = new Uint8Array(resultBuffer)

  // Parse concatenated 20-byte hashes
  const results: Uint8Array[] = []
  for (let i = 0; i < inputs.length; i++) {
    results.push(resultBytes.slice(i * 20, (i + 1) * 20))
  }

  return results
}
```

### Testing

```bash
cd packages/engine
pnpm run typecheck
pnpm run test
```

### Acceptance Criteria
- [x] IHasher interface updated with optional `sha1Batch`
- [x] DaemonHasher implements `sha1Batch`
- [x] Correct binary encoding/decoding
- [x] Falls back to single-item for length 0 or 1
- [x] Unit tests for encoding format

---

## Phase 4: Precomputed req2 Cache

**Goal:** Precompute `SHA1('req2' + infoHash)` when torrents are initialized

### Files to Modify
- `packages/engine/src/core/torrent.ts`
- `packages/engine/src/core/torrent-initializer.ts`
- `packages/engine/src/core/bt-engine.ts`

### Torrent Class Update

```typescript
// In torrent.ts

/**
 * Precomputed SHA1('req2' + infoHash) for MSE incoming connection identification.
 * Set during metadata initialization.
 */
private _req2Hash: Uint8Array | null = null

get req2Hash(): Uint8Array | null {
  return this._req2Hash
}

setReq2Hash(hash: Uint8Array): void {
  this._req2Hash = hash
}
```

### Torrent Initializer Update

```typescript
// In torrent-initializer.ts, inside initializeTorrentMetadata()

// After torrent.setMetadata(infoBuffer)
// Precompute req2 hash for MSE incoming connection identification
const req2Input = concat(encode('req2'), torrent.infoHash)
const req2Hash = await engine.hasher.sha1(req2Input)
torrent.setReq2Hash(req2Hash)
```

### BtEngine Update for Batch Precomputation

```typescript
// In bt-engine.ts

/**
 * Precompute req2 hashes for multiple torrents (used on startup).
 * Uses batch SHA1 if available for efficiency.
 */
async precomputeReq2Hashes(torrents: Torrent[]): Promise<void> {
  const needsComputation = torrents.filter(t => t.infoHash && !t.req2Hash)
  if (needsComputation.length === 0) return

  const inputs = needsComputation.map(t => concat(encode('req2'), t.infoHash))

  let hashes: Uint8Array[]
  if (this.hasher.sha1Batch && needsComputation.length > 1) {
    hashes = await this.hasher.sha1Batch(inputs)
  } else {
    hashes = await Promise.all(inputs.map(input => this.hasher.sha1(input)))
  }

  for (let i = 0; i < needsComputation.length; i++) {
    needsComputation[i].setReq2Hash(hashes[i])
  }
}
```

### Testing

```bash
pnpm run typecheck
pnpm run test
```

### Acceptance Criteria
- [x] Torrent stores precomputed `_req2Hash`
- [x] Hash computed in `initializeTorrentMetadata()`
- [x] Batch precomputation available for startup
- [x] Works for both .torrent files and magnet links (when metadata arrives)

---

## Phase 5: Update recoverInfoHash()

**Goal:** Change from O(N) iteration to O(1) map lookup

### Files to Modify
- `packages/engine/src/crypto/key-derivation.ts`
- `packages/engine/src/crypto/mse-handshake.ts`
- `packages/engine/src/crypto/mse-socket.ts`
- `packages/engine/test/crypto/key-derivation.test.ts`

### New Function Signature

```typescript
// In key-derivation.ts

/**
 * Recover info hash from MSE XOR value using precomputed req2 map.
 *
 * @param xorValue - The 20-byte XOR value from MSE PE3: HASH('req2', SKEY) XOR HASH('req3', S)
 * @param sharedSecret - The DH shared secret
 * @param req2Map - Map from hex(SHA1('req2' + infoHash)) to infoHash
 * @param sha1 - SHA1 hash function
 * @returns The matching info hash, or null if not found
 */
export async function recoverInfoHash(
  xorValue: Uint8Array,
  sharedSecret: Uint8Array,
  req2Map: Map<string, Uint8Array>,
  sha1: (data: Uint8Array) => Promise<Uint8Array>,
): Promise<Uint8Array | null> {
  // Compute req3 = SHA1('req3' + sharedSecret)
  const req3 = await sha1(concat(encode(MSE_REQ3), sharedSecret))

  // XOR to recover req2
  const req2Computed = xor(xorValue, req3)

  // O(1) lookup
  return req2Map.get(toHex(req2Computed)) ?? null
}
```

### Backward Compatibility (Optional)

If needed, keep old signature as fallback:

```typescript
/**
 * @deprecated Use Map-based overload for better performance
 */
export async function recoverInfoHashLegacy(
  xorValue: Uint8Array,
  sharedSecret: Uint8Array,
  knownInfoHashes: Uint8Array[],
  sha1: (data: Uint8Array) => Promise<Uint8Array>,
): Promise<Uint8Array | null> {
  // Old O(N) implementation for backward compatibility
  const req3 = await sha1(concat(encode(MSE_REQ3), sharedSecret))
  const req2Computed = xor(xorValue, req3)

  for (const infoHash of knownInfoHashes) {
    const expected = await sha1(concat(encode(MSE_REQ2), infoHash))
    if (arraysEqual(req2Computed, expected)) {
      return infoHash
    }
  }
  return null
}
```

### MseSocket Options Update

```typescript
// In mse-socket.ts

export interface MseSocketOptions {
  policy: EncryptionPolicy
  sha1: (data: Uint8Array) => Promise<Uint8Array>
  getRandomBytes: (length: number) => Uint8Array

  // New: precomputed map for O(1) lookup
  req2Map?: Map<string, Uint8Array>

  // Deprecated: still supported for backward compatibility
  knownInfoHashes?: Uint8Array[]
}
```

### MseHandshake Update

```typescript
// In mse-handshake.ts, processPe3AfterSync()

let infoHash: Uint8Array | null = null

if (this.options.req2Map) {
  // Fast path: O(1) lookup
  infoHash = await recoverInfoHash(
    xorValue,
    this.sharedSecret!,
    this.options.req2Map,
    this.options.sha1,
  )
} else if (this.options.knownInfoHashes) {
  // Legacy path: O(N) iteration
  infoHash = await recoverInfoHashLegacy(
    xorValue,
    this.sharedSecret!,
    this.options.knownInfoHashes,
    this.options.sha1,
  )
}
```

### Testing

Update tests in `key-derivation.test.ts`:

```typescript
describe('recoverInfoHash', () => {
  it('should find matching hash in map', async () => {
    const infoHash = randomBytes(20)
    const sharedSecret = randomBytes(96)

    // Build req2Map
    const req2Hash = await sha1(concat(encode('req2'), infoHash))
    const req2Map = new Map([[toHex(req2Hash), infoHash]])

    // Compute XOR value as peer would send
    const req2 = await sha1(concat(encode('req2'), infoHash))
    const req3 = await sha1(concat(encode('req3'), sharedSecret))
    const xorValue = xor(req2, req3)

    const result = await recoverInfoHash(xorValue, sharedSecret, req2Map, sha1)
    expect(result).toEqual(infoHash)
  })

  it('should return null for unknown hash', async () => {
    const req2Map = new Map<string, Uint8Array>()
    const xorValue = randomBytes(20)
    const sharedSecret = randomBytes(96)

    const result = await recoverInfoHash(xorValue, sharedSecret, req2Map, sha1)
    expect(result).toBeNull()
  })
})
```

### Acceptance Criteria
- [x] `recoverInfoHash()` uses Map-based O(1) lookup
- [x] Legacy array-based function available for backward compatibility
- [x] MseSocket accepts `req2Map` option
- [x] MseHandshake uses fast path when map available
- [x] All existing tests updated and passing
- [x] New tests for map-based lookup

---

## Phase 6: BtEngine Integration

**Goal:** Wire everything together in incoming connection handling

### Files to Modify
- `packages/engine/src/core/bt-engine.ts`

### Implementation

```typescript
// In bt-engine.ts

/**
 * Build req2 lookup map from torrents with precomputed hashes.
 */
private buildReq2Map(): Map<string, Uint8Array> {
  const map = new Map<string, Uint8Array>()
  for (const torrent of this.torrents) {
    if (torrent.req2Hash) {
      map.set(toHex(torrent.req2Hash), torrent.infoHash)
    }
  }
  return map
}

// In handleIncomingConnection(), update MseSocket creation:

if (shouldHandleMse && this.torrents.length > 0) {
  const req2Map = this.buildReq2Map()

  const mseSocket = new MseSocket(rawSocket, {
    policy: this.encryptionPolicy,
    req2Map,  // New: O(1) lookup
    knownInfoHashes: this.torrents.map(t => t.infoHash),  // Fallback
    sha1: (data) => this.hasher.sha1(data),
    getRandomBytes: randomBytes,
  })

  // ... rest unchanged
}
```

### Testing

```bash
pnpm run typecheck
pnpm run test
```

Integration test scenario:
1. Add multiple torrents to engine
2. Verify req2 hashes are precomputed
3. Simulate incoming MSE connection
4. Verify only 1 SHA1 call made (for req3)

### Acceptance Criteria
- [x] BtEngine builds req2Map for incoming connections
- [x] Map passed to MseSocket
- [x] Incoming connections use O(1) lookup path
- [ ] Integration tests verify reduced SHA1 calls

---

## Phase 7: Final Validation

### Performance Verification

Before:
- 10 torrents, 5 incoming connections = 50+ SHA1 HTTP calls

After:
- 10 torrents loaded (batch): 1 HTTP call (batch SHA1)
- 5 incoming connections: 5 SHA1 HTTP calls (just req3 each)

**Improvement: 90%+ reduction in SHA1 HTTP calls**

### Test Matrix

| Scenario | Before | After |
|----------|--------|-------|
| Load 10 torrents from storage | 10 SHA1 calls | 1 batch call |
| 1 incoming connection (10 torrents) | 11 SHA1 calls | 1 SHA1 call |
| 5 incoming connections (10 torrents) | 55 SHA1 calls | 5 SHA1 calls |
| Add torrent via magnet (metadata arrives) | 1 SHA1 call | 1 SHA1 call |

### Checklist

- [x] All phases complete
- [x] `pnpm run typecheck` passes
- [x] `pnpm run test` passes
- [x] `pnpm run lint` passes (only pre-existing warnings)
- [ ] `cargo build` passes (io-daemon) - Phase 1 done separately
- [ ] `./gradlew :companion-server:compileDebugKotlin` passes - Phase 2 done separately
- [ ] `./gradlew :app:compileDebugKotlin` passes
- [ ] Manual testing on Android emulator
- [ ] Manual testing with io-daemon

---

## File Summary

| File | Phase | Changes |
|------|-------|---------|
| `desktop/io-daemon/src/hashing.rs` | 1 | Add batch endpoint |
| `android/companion-server/.../NettyHttpServer.kt` | 2 | Add batch endpoint |
| `packages/engine/src/interfaces/hasher.ts` | 3 | Add `sha1Batch?` |
| `packages/engine/src/adapters/daemon/daemon-hasher.ts` | 3 | Implement `sha1Batch` |
| `packages/engine/src/core/torrent.ts` | 4 | Add `_req2Hash` |
| `packages/engine/src/core/torrent-initializer.ts` | 4 | Precompute req2 |
| `packages/engine/src/core/bt-engine.ts` | 4, 6 | Batch precompute, build map |
| `packages/engine/src/crypto/key-derivation.ts` | 5 | Map-based lookup |
| `packages/engine/src/crypto/mse-socket.ts` | 5 | Add `req2Map` option |
| `packages/engine/src/crypto/mse-handshake.ts` | 5 | Use fast path |
| `packages/engine/test/crypto/key-derivation.test.ts` | 5 | Update tests |
