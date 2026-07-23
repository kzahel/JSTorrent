# SHA1 Routing Hasher Implementation Plan

## Problem Statement

Currently, the Chrome extension uses `DaemonHasher` for all SHA1 operations, which sends HTTP requests to the daemon:
- **Desktop (Mac/Windows/Linux)**: HTTP to `127.0.0.1` - low latency (~1-5ms)
- **ChromeOS**: HTTP to `100.115.92.2` (ARC VM) - significant latency (~10-50ms per request)

This creates performance issues on ChromeOS, particularly for latency-sensitive operations like MSE handshakes (5 hashes per peer connection).

## Solution

Implement a `RoutingHasher` that routes SHA1 calls based on payload characteristics:
- **Small/latency-sensitive payloads** → `SubtleCryptoHasher` (local Web Crypto API)
- **Large payloads** → Delegate hasher (DaemonHasher or future WorkerHasher)

## SHA1 Usage Analysis

| Reason | Typical Size | Latency Sensitivity | Target |
|--------|--------------|---------------------|--------|
| `mse-init` | ~40-100 bytes × 5 | **High** | SubtleCrypto |
| `mse-resp` | ~40-100 bytes | **High** | SubtleCrypto |
| `mse-resp-req1` | ~40 bytes | **High** | SubtleCrypto |
| `mse-resp-req3` | ~60 bytes | **High** | SubtleCrypto |
| `mse-resp-check` | ~60 bytes | **High** | SubtleCrypto |
| `mse-resp-keys` | ~100 bytes × 2 | **High** | SubtleCrypto |
| `mse-req2` | ~60 bytes | **High** | SubtleCrypto |
| `info-hash` | ~1-100KB | Medium | SubtleCrypto |
| `metadata-verify` | ~1-100KB | Medium | SubtleCrypto |
| `piece-verify` | 16KB - 32MB | Low | Delegate |
| `piece-upload-verify` | 16KB - 32MB | Low | Delegate |
| `torrent-create` | 16KB - 32MB | Low | Delegate |

---

## Phase 1: Add Typed SHA1 Reasons

**Goal**: Add type safety to SHA1 reason strings and define routing categories.

### Changes

1. **Update `packages/engine/src/interfaces/hasher.ts`**:
   ```typescript
   /**
    * Reasons for SHA1 hashing - used for routing and debugging.
    */
   export type Sha1Reason =
     // MSE handshake (small, latency-sensitive)
     | 'mse-init'
     | 'mse-resp'
     | 'mse-resp-req1'
     | 'mse-resp-req3'
     | 'mse-resp-check'
     | 'mse-resp-keys'
     | 'mse-req2'
     // Metadata (small-medium)
     | 'info-hash'
     | 'metadata-verify'
     // Piece operations (large)
     | 'piece-verify'
     | 'piece-upload-verify'
     | 'torrent-create'

   /**
    * Reasons that should use local (SubtleCrypto) hashing when available.
    * These are small payloads where HTTP latency would dominate.
    */
   export const SUBTLE_CRYPTO_REASONS: ReadonlySet<Sha1Reason> = new Set([
     'mse-init',
     'mse-resp',
     'mse-resp-req1',
     'mse-resp-req3',
     'mse-resp-check',
     'mse-resp-keys',
     'mse-req2',
     'info-hash',
     'metadata-verify',
   ])

   export interface IHasher {
     sha1(data: Uint8Array, reason?: Sha1Reason): Promise<Uint8Array>
     sha1Batch?(inputs: Uint8Array[], reason?: Sha1Reason): Promise<Uint8Array[]>
   }
   ```

2. **Update all hasher implementations** to use `Sha1Reason` type:
   - `SubtleCryptoHasher`
   - `DaemonHasher`
   - `NativeHasher`
   - `NodeHasher`

3. **Update all call sites** to use typed reasons (should be no-op if strings match).

### Verification

```bash
pnpm run typecheck
pnpm run test
```

All existing tests should pass. Type errors will surface any mistyped reason strings.

---

## Phase 2: Implement RoutingHasher

**Goal**: Create a hasher that routes based on reason and payload size.

### Changes

1. **Create `packages/engine/src/adapters/browser/routing-hasher.ts`**:
   ```typescript
   import { IHasher, Sha1Reason, SUBTLE_CRYPTO_REASONS } from '../../interfaces/hasher'
   import { SubtleCryptoHasher } from './subtle-crypto-hasher'

   /**
    * Routes SHA1 calls based on reason/size:
    * - Small/latency-sensitive → SubtleCrypto (local, fast)
    * - Large payloads → delegate hasher (Daemon or Worker)
    */
   export class RoutingHasher implements IHasher {
     private subtleHasher: SubtleCryptoHasher | null
     private delegateHasher: IHasher

     // Size threshold for unknown reasons
     private static readonly SIZE_THRESHOLD = 64 * 1024  // 64KB

     constructor(delegateHasher: IHasher) {
       this.delegateHasher = delegateHasher
       this.subtleHasher = crypto?.subtle ? new SubtleCryptoHasher() : null
     }

     async sha1(data: Uint8Array, reason?: Sha1Reason): Promise<Uint8Array> {
       if (this.shouldUseSubtle(data.length, reason)) {
         return this.subtleHasher!.sha1(data, reason)
       }
       return this.delegateHasher.sha1(data, reason)
     }

     async sha1Batch(inputs: Uint8Array[], reason?: Sha1Reason): Promise<Uint8Array[]> {
       // MSE batches are always small - use SubtleCrypto
       if (this.subtleHasher && reason?.startsWith('mse')) {
         return Promise.all(inputs.map((i) => this.subtleHasher!.sha1(i, reason)))
       }
       // Large batches - use delegate
       if (this.delegateHasher.sha1Batch) {
         return this.delegateHasher.sha1Batch(inputs, reason)
       }
       return Promise.all(inputs.map((i) => this.delegateHasher.sha1(i, reason)))
     }

     private shouldUseSubtle(size: number, reason?: Sha1Reason): boolean {
       if (!this.subtleHasher) return false
       if (reason && SUBTLE_CRYPTO_REASONS.has(reason)) return true
       return size < RoutingHasher.SIZE_THRESHOLD
     }
   }
   ```

2. **Export from index**:
   ```typescript
   // packages/engine/src/index.ts
   export { RoutingHasher } from './adapters/browser/routing-hasher'
   ```

3. **Add unit tests** for `RoutingHasher`:
   - Verify small payloads route to SubtleCrypto
   - Verify large payloads route to delegate
   - Verify MSE reasons always use SubtleCrypto
   - Verify fallback when SubtleCrypto unavailable

### Verification

```bash
pnpm run typecheck
pnpm run test
```

New tests should pass. Existing tests unaffected (RoutingHasher not yet integrated).

---

## Phase 3: Integrate into Extension

**Goal**: Use RoutingHasher in the Chrome extension.

### Changes

1. **Update `packages/client/src/engine-manager/chrome-extension-engine-manager.ts`**:
   ```typescript
   import {
     // ... existing imports
     DaemonHasher,
     RoutingHasher,
   } from '@jstorrent/engine'

   // In doInit(), replace:
   // const hasher = new DaemonHasher(this.daemonConnection)

   // With:
   const daemonHasher = new DaemonHasher(this.daemonConnection)
   const hasher = new RoutingHasher(daemonHasher)
   ```

### Verification

1. **Build and test on desktop**:
   ```bash
   cd extension && pnpm build
   # Load extension, add torrent, verify peer connections work
   # Check console for any SHA1-related errors
   ```

2. **Test on ChromeOS**:
   ```bash
   ./scripts/deploy-chromebook.sh
   # Add torrent with encrypted peers
   # Verify MSE handshakes complete faster
   # Monitor latency improvement
   ```

3. **Specific test cases**:
   - [ ] MSE handshake with encrypted peer succeeds
   - [ ] Piece verification works correctly
   - [ ] Torrent metadata parsing works
   - [ ] No console errors related to hashing

---

## Phase 4: Remove SubtleCryptoHasher.sha1Batch (Cleanup)

**Goal**: Remove redundant batch implementation from SubtleCryptoHasher.

### Rationale

`SubtleCryptoHasher.sha1Batch` just does `Promise.all`, which is identical to `BtEngine`'s fallback. Since SubtleCrypto has no native batch API, remove it to clarify that `sha1Batch` should only be implemented when there's a real optimization.

### Changes

1. **Update `packages/engine/src/adapters/browser/subtle-crypto-hasher.ts`**:
   ```typescript
   export class SubtleCryptoHasher implements IHasher {
     async sha1(data: Uint8Array, _reason?: Sha1Reason): Promise<Uint8Array> {
       // ... existing implementation
     }
     // Remove sha1Batch - let callers use Promise.all fallback
   }
   ```

### Verification

```bash
pnpm run typecheck
pnpm run test
```

---

## Phase 5 (Future): Web Worker Hasher for Large Payloads

**Goal**: Offload large piece hashing to a dedicated Web Worker.

### Motivation

For piece verification (16KB - 32MB), hashing on the main thread could cause jank. A dedicated worker with transferables provides:
- Zero-copy buffer transfer
- Main thread stays responsive
- Could parallelize across multiple workers

### Design

```
Main Thread                     Worker Thread
    |                               |
    |  postMessage(buffer)  ------> |
    |  [transferable]               |
    |                               |  crypto.subtle.digest()
    |                               |
    |  <------ postMessage(hash)    |
    |          [transferable]       |
```

### Implementation

1. **Create `packages/engine/src/adapters/browser/hash-worker.ts`**:
   ```typescript
   // Web Worker script
   self.onmessage = async (e: MessageEvent<{ id: number; data: ArrayBuffer }>) => {
     const { id, data } = e.data
     const hash = await crypto.subtle.digest('SHA-1', data)
     self.postMessage({ id, hash }, [hash])
   }
   ```

2. **Create `packages/engine/src/adapters/browser/worker-hasher.ts`**:
   ```typescript
   export class WorkerHasher implements IHasher {
     private worker: Worker
     private pending = new Map<number, (hash: Uint8Array) => void>()
     private nextId = 0

     constructor() {
       this.worker = new Worker(new URL('./hash-worker.ts', import.meta.url))
       this.worker.onmessage = (e) => {
         const { id, hash } = e.data
         this.pending.get(id)?.(new Uint8Array(hash))
         this.pending.delete(id)
       }
     }

     async sha1(data: Uint8Array, _reason?: Sha1Reason): Promise<Uint8Array> {
       const id = this.nextId++
       const buffer = data.buffer.slice(
         data.byteOffset,
         data.byteOffset + data.byteLength
       )
       return new Promise((resolve) => {
         this.pending.set(id, resolve)
         this.worker.postMessage({ id, data: buffer }, [buffer])
       })
     }
   }
   ```

3. **Update RoutingHasher to use WorkerHasher** as delegate on ChromeOS.

### Considerations

- Worker initialization has startup cost (~10-50ms)
- Need to handle worker errors/crashes
- May want worker pool for parallel hashing
- Bundle configuration needed for worker script

### Verification

- Benchmark piece verification latency
- Verify main thread responsiveness during large downloads
- Test worker recovery after crash

---

## Summary

| Phase | Description | Risk | Effort |
|-------|-------------|------|--------|
| 1 | Add typed Sha1Reason | Low | Small |
| 2 | Implement RoutingHasher | Low | Medium |
| 3 | Integrate into extension | Medium | Small |
| 4 | Cleanup SubtleCryptoHasher | Low | Trivial |
| 5 | Web Worker hasher | Medium | Large |

Phases 1-4 can be done incrementally with verification at each step. Phase 5 is optional optimization for later.
