# Align Piece Request Handling with libtorrent

## Problem

Pieces get stuck for 2+ minutes with the same peer due to a cycle:
1. Stale requests cancelled → exclusive peer cleared
2. Same peer reclaims exclusive next tick → blocks re-requested from same slow peer
3. `shouldAbandon()` never triggers because progress > 50%

More broadly, our piece/request handling diverges from libtorrent in several ways that hurt multi-peer performance.

## Phases

### Phase 0: Break the stuck-piece cycle (immediate fix)

**Goal**: Fix the bug visible in the screenshot — pieces stuck 2+ minutes cycling with one peer.

**Changes**:

1. **`active-piece.ts`**: Add `_failedPeers: Set<string>`. When `cancelRequest()` clears exclusive because a peer timed out, add that peer to `_failedPeers`. In `canRequestFrom()`, reject peers in `_failedPeers` (they can't request from this piece anymore). Add `clearFailedPeer(peerId)` for recovery if peer sends data later.

2. **`torrent-tick-loop.ts`**: In `cleanupStuckPieces()`, after cancelling stale requests, if the piece is >60s old with outstanding stale requests from the same peer, add them to failed peers. Remove `shouldAbandon()` calls entirely — never abandon pieces with received data.

3. **`active-piece.ts`**: Remove `shouldAbandon()` method. libtorrent never abandons pieces; it just releases individual blocks.

**Files**: `active-piece.ts`, `torrent-tick-loop.ts`
**Test**: `active-piece.test.ts`, `active-piece-manager.test.ts`, manual test with Ubuntu torrent
**Risk**: Low — only adds restrictions on which peer can request. LAN single-peer unaffected (no timeouts).

---

### Phase 1: Snubbing & RTT-based timeouts

**Goal**: Replace fixed 10s timeout with libtorrent's adaptive RTT-based timeout. Add peer snubbing.

**Changes**:

1. **`peer-connection.ts`**: Add RTT tracking:
   - `_rttSamples: SlidingAverage` (20-sample window, track mean + deviation)
   - `_snubbed: boolean` — when true, pipeline capped at 1
   - `requestTimeout(): number` — returns `avg + 4σ` (min 2s, max 60s), or 60s if no samples
   - `snub()`: set `_snubbed = true`, cap pipeline to 1, exit slow-start
   - `recordRttSample(ms)`: called when block received, feeds sliding average
   - When snubbed peer sends data: `_snubbed = false` (recovery)

2. **`peer-connection.ts`**: Add `_requestedAt: number` — timestamp of oldest in-flight request. Reset when block received and more requests pending. Used by timeout checker.

3. **`torrent-tick-loop.ts`**: Replace `BLOCK_REQUEST_TIMEOUT_MS = 10_000` with per-peer `peer.requestTimeout()`. In `cleanupStuckPieces()`:
   - For each peer with `now - peer.requestedAt > peer.requestTimeout()`: call `snubPeer(peer)`
   - `snubPeer()` logic (from libtorrent):
     a. Set `peer.snub()`
     b. Clear unsent requests for this peer from all pieces
     c. For ONE in-flight request: check `freeBlocks` first (see Phase 2)

4. **`torrent.ts`** or **`torrent-peer-handler.ts`**: When block received, compute RTT sample and call `peer.recordRttSample()`.

**Files**: `peer-connection.ts`, `torrent-tick-loop.ts`, `torrent-peer-handler.ts`
**Test**: Unit test for SlidingAverage, test snub/unsnub cycle
**Risk**: Medium — timeout behavior changes. Verify LAN peer never gets snubbed (RTT should be <50ms → timeout ~2s, plenty of margin).
**Depends on**: Nothing (can be done before or after Phase 0)

---

### Phase 2: free_blocks check & smart cancellation

**Goal**: Don't cancel stale requests blindly. Only cancel when the block is the last thing blocking piece completion (libtorrent's core insight).

**Changes**:

1. **`active-piece.ts`**: Add `get freeBlocks(): number` — count of blocks that are neither received, nor requested, nor writing. This is `_unrequestedCount` (already tracked).

2. **`torrent-tick-loop.ts`**: Rewrite stale request handling in `cleanupStuckPieces()`:
   - For each piece, for each timed-out request:
     - Compute `freeBlocks = piece.unrequestedCount` (blocks available for other peers)
     - **If freeBlocks > 0**: Don't cancel this request. Other peers can still pick up free blocks. Just snub the peer.
     - **If freeBlocks == 0**: This block is blocking piece completion. Cancel it (via `piece.cancelRequest()`), send CANCEL to peer. Try to request replacement from another peer.
   - This prevents unnecessary churn when pieces have plenty of free blocks.

3. **Remove the demotion-then-immediate-reclaim cycle**: With the free_blocks check, we only cancel blocks that truly need redistribution. Combined with Phase 0's failed-peer tracking, the cancelled block goes to a different peer.

**Files**: `active-piece.ts`, `torrent-tick-loop.ts`
**Test**: Unit test: piece with 1 free block should not have other stale requests cancelled. Piece with 0 free blocks should have the stale request cancelled.
**Risk**: Low — strictly more conservative about cancellation.
**Depends on**: Phase 1 (needs snubbing)

---

### Phase 3: Slow-start queue sizing

**Goal**: Replace aggressive `+150/sec` pipeline ramp with libtorrent's slow-start algorithm.

**Changes**:

1. **`peer-connection.ts`**: Replace `recordBlockReceived()` adaptive logic:
   - `_slowStart: boolean = true` — start in slow-start mode
   - In slow-start: `pipelineDepth += 1` per block received (start from `MIN_PIPELINE_DEPTH = 2`)
   - Exit slow-start when: download rate increase < 5KB/s over 1 second, or peer snubbed
   - Normal mode: `pipelineDepth = queueTime * downloadRate / BLOCK_SIZE`
     - `queueTime` = 3s (configurable, libtorrent's `request_queue_time`)
     - This sizes the queue to keep the pipe full for the RTT × bandwidth product
   - Snubbed mode: `pipelineDepth = 1`
   - On unsnub: re-enter slow-start

2. **`peer-connection.ts`**: Track `_downloadedLastSecond` and `_lastSecondDownload` for rate plateau detection.

**Files**: `peer-connection.ts`
**Test**: Unit test slow-start ramp, plateau detection, snub→1→unsnub→slow-start
**Risk**: Medium — directly affects throughput. Must verify LAN still reaches full speed. Slow-start from 2 to 500 takes ~500 blocks = ~8MB at 16KB/block. At 90 MB/s LAN that's <0.1s. Should be fine.
**Depends on**: Phase 1 (snubbing)

---

### Phase 4: Remove exclusive ownership → open pieces

**Goal**: Remove `_exclusivePeer` entirely. Any peer can request any block from any piece, matching libtorrent.

**Changes**:

1. **`active-piece.ts`**:
   - Remove `_exclusivePeer`, `claimExclusive()`, `clearExclusivePeer()`, `canRequestFrom()`
   - Remove `_failedPeers` from Phase 0 (no longer needed without exclusivity)

2. **`piece-requester.ts`**:
   - Remove all `canRequestFrom()` checks
   - Remove all `claimExclusive()` calls
   - Remove `peerIsFast` parameter threading
   - Add soft affinity as tiebreaker: when multiple partial pieces are available, prefer pieces where this peer already has blocks in-flight (contiguity preference, like libtorrent's `requested_from()`). This is a sort preference, NOT a hard lock.

3. **`torrent-tick-loop.ts`**: Remove exclusive-peer-related cleanup from `cleanupStuckPieces()`.

4. **`active-piece-manager.ts`**: Remove exclusive peer handling from `clearRequestsForPeer()`.

**Files**: `active-piece.ts`, `piece-requester.ts`, `torrent-tick-loop.ts`, `active-piece-manager.ts`
**Test**: All existing tests must pass (with exclusive ownership assertions removed). Manual test: Ubuntu torrent with 10+ peers should show pieces completing faster with blocks from multiple peers.
**Risk**: Medium-high — changes core request strategy. Possible regression: more piece fragmentation (blocks from many peers = more memory for partial pieces). Mitigated by `maxActivePieces: 64` cap and `shouldPrioritizePartials()` cap. Verify LAN throughput unchanged (single peer = no fragmentation either way).
**Depends on**: Phase 0 (Phase 0 adds _failedPeers as stopgap; Phase 4 removes it along with exclusive)

---

### Phase 5: Piece-level no-data timeout

**Goal**: Add libtorrent's `piece_timeout` — if no data arrives for a piece for 20s, snub the requesting peer.

**Changes**:

1. **`active-piece.ts`**: `_lastActivity` already exists. Add `getRequestingPeers(): Set<string>` — returns all peers with outstanding requests on this piece.

2. **`torrent-tick-loop.ts`**: In `cleanupStuckPieces()`, add piece-level timeout check:
   - For each active piece (partial + fullyRequested):
     - If `now - piece.lastActivity > PIECE_NO_DATA_TIMEOUT_MS` (20s):
       - For each peer requesting blocks on this piece: `snubPeer(peer)`
       - This is more aggressive than per-block timeout — catches peers that accept requests but don't send data

**Files**: `active-piece.ts`, `torrent-tick-loop.ts`
**Test**: Unit test: piece with no activity for 20s should trigger snub on requesting peers
**Risk**: Low — additive check. LAN peers will have <1s between blocks.
**Depends on**: Phase 1 (snubbing)

---

### Phase 6: Better choke/disconnect cleanup

**Goal**: Align choke and disconnect handling with libtorrent.

**Changes**:

1. **`torrent-peer-handler.ts`** `handleChoke()`: Currently clears ALL requests for the peer. Change to only clear requests from `m_request_queue` equivalent (unsent). In-flight requests (already sent) should remain — the peer may still send them. (Note: BitTorrent spec says choke discards requests, so this may need nuance per-implementation.)

2. **`torrent.ts`** `removePeer()`: Already calls `clearRequestsForPeer()`. Verify that this properly handles all states (partial, fullyRequested) and triggers appropriate demotions. Currently looks correct.

**Files**: `torrent-peer-handler.ts`, `torrent.ts`
**Test**: Manual test: disconnect peer during download, verify blocks are immediately re-requested from others
**Risk**: Low — mostly verification of existing behavior
**Depends on**: Phase 2 (free_blocks understanding)

---

## Dependency Graph

```
Phase 0 (stopgap fix) ────────────────────────→ Phase 4 (remove exclusive)
                                                      ↑
Phase 1 (snub + RTT) ──→ Phase 2 (free_blocks) ──────┘
         │
         ├──→ Phase 3 (slow-start)
         │
         └──→ Phase 5 (piece timeout)

Phase 6 (choke/disconnect) — independent, do after Phase 2
```

**Recommended order**: 0 → 1 → 2 → 3 → 4 → 5 → 6

Phase 0 is the immediate bug fix. After Phase 2, the core stuck-piece issue is fully resolved with proper libtorrent-style handling. Phases 3-6 are refinements.

## Verification

After each phase:
1. `pnpm run typecheck && pnpm run test` — no regressions
2. LAN benchmark: `./scripts/benchmark-daemon-download.sh` — throughput should remain ~90 MB/s
3. Ubuntu torrent manual test: 10+ peers, monitor via extension Pieces tab
   - No pieces stuck >30s
   - Active piece count stays bounded
   - Throughput improves (target: sustain 5+ MB/s without degradation)

## Key Files

| File | Phases |
|------|--------|
| `packages/engine/src/core/active-piece.ts` | 0, 2, 4, 5 |
| `packages/engine/src/core/peer-connection.ts` | 1, 3 |
| `packages/engine/src/core/torrent-tick-loop.ts` | 0, 1, 2, 5 |
| `packages/engine/src/core/piece-requester.ts` | 4 |
| `packages/engine/src/core/active-piece-manager.ts` | 4 |
| `packages/engine/src/core/torrent-peer-handler.ts` | 1, 6 |
| `packages/engine/src/core/torrent.ts` | 1, 6 |
| `packages/engine/test/core/active-piece.test.ts` | 0, 2, 4 |
| `packages/engine/test/core/active-piece-manager.test.ts` | 4 |
