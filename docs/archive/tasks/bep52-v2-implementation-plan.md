# BitTorrent v2 (BEP 52) Implementation Plan

## Context

JSTorrent is currently v1-only. Adding v2 support is a competitive differentiator — we'd be the only v2-capable client on Chrome, ChromeOS, and Android. v2 also provides SHA-256 collision resistance (hedges against SHA-1 attacks for piece poisoning/DOS). The spec is at `docs/beps_md/draft/bep_0052.md`. Reference implementations at `~/code/reference/qBittorrent/` (delegates v2 to libtorrent, but InfoHash wrapper, magnet parsing, and dual-hash indexing patterns are useful).

## Phased Approach

Five phases, each independently testable. Phases 0-1 are the MVP (download v2 .torrent files). Later phases add magnet support, performance, creation, and full hybrid swarm participation.

---

## Phase 0: Foundation (SHA256 + types, no behavior change)

All existing tests must still pass unchanged.

### 0.1 IHasher — add SHA256

**`packages/engine/src/interfaces/hasher.ts`**
- Add `Sha256Reason` type: `'v2-info-hash' | 'v2-piece-verify' | 'v2-merkle-leaf' | 'v2-merkle-node' | 'v2-metadata-verify'`
- Add to `IHasher`: `sha256(data: Uint8Array, reason?: Sha256Reason): Promise<Uint8Array>`
- Add optional: `sha256Batch?(inputs: Uint8Array[], reason?: Sha256Reason): Promise<Uint8Array[]>`

**Implement in all hasher backends:**
- `adapters/node/node-hasher.ts` — `crypto.createHash('sha256')`
- `adapters/browser/subtle-crypto-hasher.ts` — `crypto.subtle.digest('SHA-256', data)`
- `adapters/browser/worker-hasher.ts` — extend worker message protocol
- `adapters/browser/transferring-worker-hasher.ts` — extend for SHA256
- `adapters/browser/routing-hasher.ts` — route SHA256 same as SHA1
- `adapters/daemon/daemon-hasher.ts` — new daemon endpoint or parameterize existing
- `adapters/native/native-hasher.ts` — new JNI binding

**Rust io-daemon** (`desktop/io-daemon/`): Add SHA256 hash endpoint.
**Android** (`android/io-core/`): Add SHA256 JNI hash function + register in FileBindings.

### 0.2 InfoHash v2 types

**`packages/engine/src/utils/infohash.ts`** — add alongside existing (no changes to existing functions):
- `InfoHashV2Hex` branded type (64-char lowercase hex)
- `infoHashV2FromHex(hex: string): InfoHashV2Hex`
- `infoHashV2FromBytes(bytes: Uint8Array): InfoHashV2Hex` (validates 32 bytes)
- `truncateV2ToV1Bytes(v2Hash: Uint8Array): Uint8Array` (first 20 bytes)
- `looksLikeBareInfoHashV2(input: string): boolean` (64 hex chars)

### 0.3 TorrentVersion type

**New file: `packages/engine/src/core/torrent-version.ts`**
```typescript
type TorrentVersion = 'v1' | 'v2' | 'hybrid'

interface TorrentIdentity {
  version: TorrentVersion
  v1Hash?: Uint8Array        // 20-byte SHA1 (v1, hybrid)
  v2Hash?: Uint8Array        // 32-byte SHA256 (v2, hybrid)
  v2HashTruncated?: Uint8Array  // First 20 bytes of v2Hash (tracker/DHT/handshake)
  canonicalHash: Uint8Array     // v1Hash if present, else v2HashTruncated
  canonicalHex: InfoHashHex     // Hex of canonicalHash (for indexing)
}
```

**Identity model**: Use v1 hash as canonical key when present (hybrid). Use truncated v2 for v2-only. This matches tracker/DHT (always 20-byte) and simplifies session persistence.

---

## Phase 1: Parse and Download v2/Hybrid .torrent Files

### 1.1 Merkle tree module

**New file: `packages/engine/src/core/merkle-tree.ts`**
- `MERKLE_LEAF_SIZE = 16384` (16 KiB, fixed by spec)
- `ZERO_HASH = new Uint8Array(32)` (padding beyond EOF)
- `computeMerkleRoot(leafHashes: Uint8Array[]): Uint8Array` — binary SHA256 tree, pad with ZERO_HASH to power-of-2
- `computeFileLeafHashes(data: Uint8Array, hasher: IHasher): Promise<Uint8Array[]>` — hash 16 KiB blocks
- `verifyPieceLayers(piecesRoot: Uint8Array, layerHashes: Uint8Array[], pieceLength: number, fileLength: number): boolean`
- `extractPieceHashes(layerHashes: Uint8Array[], pieceLength: number): Uint8Array[]` — get per-piece hashes from piece layer

Pure computation, no I/O. Highly testable with known vectors.

### 1.2 v2 file tree parsing

**`packages/engine/src/core/torrent-parser.ts`**
- Detect `info['meta version'] === 2` → v2 or hybrid
- If v2/hybrid: parse `file tree` nested dict:
  - Walk recursively; `""` key with `length` = file entry
  - Extract `pieces root` (32-byte) from each file
  - Calculate offsets with v2 alignment (each non-empty file starts on piece boundary)
  - Sanitize path components (reject `..`, `.`)
- Parse `piece layers` from top-level dict (NOT inside info):
  - Map: merkle root bytes → concatenated piece-layer hashes
  - Validate each against its file's `pieces root`
- Compute v2 info hash: `await hasher.sha256(infoBuffer, 'v2-info-hash')`
- For hybrid: also parse v1 fields (existing code path)

**Extend `ParsedTorrent`:**
```typescript
interface ParsedTorrent {
  // existing fields...
  version: TorrentVersion
  v2InfoHash?: Uint8Array
  v2PieceHashes?: Map<string, Uint8Array[]>  // merkle root hex -> per-piece SHA256 hashes
  v2FileTree?: V2FileEntry[]
}

interface V2FileEntry {
  path: string
  length: number
  piecesRoot?: Uint8Array  // 32-byte merkle root (absent for empty files)
}
```

### 1.3 v2 piece mapping

**New file: `packages/engine/src/core/v2-piece-map.ts`**
- In v2, each non-empty file is aligned to piece boundary. Piece address space differs from v1's flat concatenation.
- `V2PieceMap`: given file tree + piece length, computes piece index → file + offset
- Handles alignment gaps between files
- For hybrid: validate v1 and v2 piece counts match (accounting for BEP 47 padding)

### 1.4 Torrent class extensions

**`packages/engine/src/core/torrent.ts`**
- Add `version: TorrentVersion` (default `'v1'`)
- Add `v2InfoHash?: Uint8Array`, `v2PieceHashes?: Map<string, Uint8Array[]>`
- `verifyPiece()`: branch on version — v1 uses SHA1, v2 uses SHA256 per-piece hash from piece layers
- `getPieceHash()`: return appropriate hash based on version
- For hybrid: verify both hashes (BEP 52 requires this)

### 1.5 Peer connection — stop rejecting v2

**`packages/engine/src/core/peer-connection.ts`**
- Remove the disconnect-on-`info_hash2` logic (lines 686-702)
- Instead: store `peerV2Hash` for future use, log it
- For hybrid: set v2 support bit in reserved bytes (byte 7, bit 0x10)
- Handshake still uses 20-byte hash (v1 for hybrid, truncated v2 for v2-only)

### 1.6 Wire protocol — v2 message types

**`packages/engine/src/protocol/wire-protocol.ts`**
- Add `MessageType.HASH_REQUEST = 21`, `HASHES = 22`, `HASH_REJECT = 23`
- Add `V2_RESERVED = { byte: 7, mask: 0x10 }`
- `createHandshake()`: accept optional v2 flag
- `parseHandshake()`: detect and return v2 support flag

### 1.7 Other touched files

- `torrent-initializer.ts` — handle v2/hybrid parsed torrents, set version + v2 fields
- `torrent-factory.ts` — detect version in `parseTorrentInput()` for .torrent files
- `torrent-content-storage.ts` — v2 file alignment for piece-to-file mapping
- `session-persistence.ts` — store version, v2 info hash, piece layers in persisted state

### 1.8 Testing

- Generate test fixtures: `packages/engine/test/fixtures/v2-test.torrent` and `hybrid-test.torrent` using qBittorrent or libtorrent Python bindings
- Unit tests: merkle tree computation against known vectors
- Unit tests: torrent parser with v2-only and hybrid .torrent files
- Unit tests: v2 piece map alignment edge cases (empty files, files < piece length, exact alignment)
- Integration test: download v2 torrent from qBittorrent seeder

---

## Phase 2: v2 Magnet Links + Hash Request Protocol

### 2.1 Magnet parsing

**`packages/engine/src/utils/magnet.ts`**
- `parseMagnet()`: accept `urn:btmh:1220{64hex}` (multihash: 0x12=SHA256, 0x20=32 bytes)
- Support hybrid magnets with both `urn:btih:` and `urn:btmh:`
- Add `v2InfoHash?: InfoHashV2Hex` to `ParsedMagnet`
- `generateMagnet()`: produce appropriate xt params based on version

### 2.2 Metadata fetcher — SHA256 verification

**`packages/engine/src/core/metadata-fetcher.ts`**
- For v2 magnet: verify fetched metadata with SHA256 instead of SHA1

### 2.3 Hash request protocol

**New file: `packages/engine/src/core/hash-fetcher.ts`**
- After metadata fetched for v2 magnet, request piece layer hashes from peers via message type 21
- Build requests: for each file's pieces root, request piece-layer hashes
- Verify received hashes against merkle root
- Emit event when all piece layers ready

### 2.4 Torrent state machine

- `no_metadata` → `has_metadata` → `has_piece_layers` → `downloading`
- New intermediate state for v2 magnets

---

## Phase 3: Performance — Backend SHA256 verifyChunks

### 3.1 VerifyChunksRequest extension

**`packages/engine/src/interfaces/filesystem.ts`**
- Add `hashAlgorithm?: 'sha1' | 'sha256'` (default `'sha1'`)
- Add `hashSize?: number` (20 or 32)

### 3.2 All backends

Update verifyChunks in all 6 TS adapters + Rust io-daemon + Android FileManager to support SHA256 with configurable hash stride. Phase 1 uses per-piece fallback; this phase enables fast batch recheck for v2.

---

## Phase 4: v2 Torrent Creation + Upload

- `torrent-creator.ts`: produce v2/hybrid .torrent files (SHA256 merkle trees, file tree, piece layers)
- Respond to hash request messages (type 21) when seeding v2 torrents

---

## Phase 5: Full Hybrid Swarm Participation

- Announce with both hashes to trackers/DHT
- Connection upgrade via reserved bit + `info_hash2` in extended handshake
- Track per-peer v1 vs v2 capability
- MSE: register both req2 hashes for hybrid torrents

---

## Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Torrent identity key | v1 hash when present, truncated v2 otherwise | Matches tracker/DHT (always 20-byte). Simplest. qBittorrent does similar. |
| Phase 1 piece verify | Per-piece SHA256 in TS (no backend changes) | Ships faster. Backend SHA256 in Phase 3 for performance. |
| v2 magnet support | Phase 2 (not Phase 1) | .torrent files include piece layers. Magnets need hash request protocol — more complex. |
| v2 creation | Phase 4 (deferred) | Download is the priority use case. |

## Highest Risk

**v2 file-to-piece alignment**. In v1, files are concatenated and pieces span boundaries. In v2, each file starts on a piece boundary with alignment gaps. `TorrentContentStorage.write()`/`read()` assume v1 concatenation. Getting this wrong corrupts downloads. Mitigation: exhaustive unit tests + test against known v2 torrents from qBittorrent.

## Verification

After each phase:
1. `pnpm run typecheck && pnpm run test && pnpm run lint`
2. `cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace` (if Rust changed)
3. `./gradlew :app:compileDebugKotlin` (if Android changed)
4. Manual test: download a v2 .torrent from qBittorrent seeder, verify data integrity
