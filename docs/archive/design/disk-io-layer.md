# Disk I/O Layer Design

## Goal

Introduce a **disk-scoped I/O layer** between torrent content storage and platform file handles. The disk layer treats the physical disk as the unit of coordination — not the torrent, not the storage root. Multiple storage roots on the same physical disk share one I/O coordinator. Roots on different disks get independent coordinators that can operate in true parallel.

## Motivation

Today, each torrent manages its own disk operations via `TorrentContentStorage` → `IDiskQueue` → `IFileHandle`. There is no shared coordination at the disk level:

- **No cross-torrent prioritization**: A downloading torrent competes equally with a seeding torrent for the same disk, even though download writes are more latency-sensitive than upload reads.
- **No disk-scoped backpressure**: If the disk is slow, pieces buffer unbounded in JS memory. The `sendBufferWatermark` only covers upload-side flow control.
- **No store buffer**: Data written to disk must be re-read for upload or hash verification. Libtorrent's store buffer serves recently-written data from memory, avoiding disk round-trips.
- **No metadata fencing**: Deleting a torrent's files can race with in-flight reads/writes for that torrent.
- **No disk identification**: `StorageRootManager` doesn't know which roots share a physical disk. Two roots on the same SSD get separate queues as if they were independent devices.

## Design Principles

1. **Disk is a shared resource** — the I/O coordinator is a service, not a per-torrent utility.
2. **JS layer owns coordination** — even though actual I/O crosses FFI/HTTP boundaries, the JS engine decides ordering, priority, and backpressure.
3. **Platform layer reports disk identity** — each platform provides a `diskId` for each storage root. The JS layer groups roots by `diskId`.
4. **Incremental delivery** — each phase is independently valuable and testable. No big-bang rewrite.

## Architecture

```
┌────────────────────────────────────────────────────┐
│  Torrent                                            │
│  (piece selection, peer management)                 │
├────────────────────────────────────────────────────┤
│  DiskIO  (one per physical disk)                    │
│  • Job queue with priority (download > upload)      │
│  • Store buffer (read-from-pending-writes)          │
│  • Write buffer watermark + backpressure            │
│  • Per-torrent fencing for metadata ops             │
│  • Disk stats (throughput, queue depth, latency)    │
├────────────────────────────────────────────────────┤
│  TorrentContentStorage (per-torrent, unchanged)     │
│  • Piece → file offset mapping                      │
│  • File allocation / sparse handling                │
├────────────────────────────────────────────────────┤
│  Platform Backend (unchanged)                       │
│  • Android: NativeBatchingDiskQueue → FFI           │
│  • Rust daemon: HTTP file endpoints                 │
│  • Node.js: direct fs                               │
│  • Extension: Memory / OPFS                         │
└────────────────────────────────────────────────────┘
```

## Disk Identification by Platform

### Android (SAF)

The SAF tree URI encodes the storage volume:

```
content://com.android.externalstorage.documents/tree/primary%3ADownload
  → volumeId = "primary" (internal storage)

content://com.android.externalstorage.documents/tree/0815-4711%3A
  → volumeId = "0815-4711" (SD card)
```

Extract via `DocumentsContract.getTreeDocumentId(uri).substringBefore(':')`. This is a stable API — the volume ID prefix is a contract of the external storage documents provider.

Optionally enrich with `StorageManager.getStorageVolumes()` for free space and removable status.

### Rust (macOS/Linux/Windows)

**Unix**: `std::fs::metadata(path).dev()` returns `st_dev`, a device number unique per mounted filesystem. Two paths with the same `st_dev` share a physical disk.

**Windows**: `GetVolumePathNameW` → `GetVolumeNameForVolumeMountPointW` gives a stable volume GUID.

Exposed as a new field on the `DownloadRoot` struct, populated when a root is added.

### Node.js

`fs.statSync(path).dev` — same `st_dev` as Rust on Unix. On Windows, use `os.networkInterfaces` is not helpful; a small native addon or `child_process` call to `wmic` may be needed, but Node.js on Windows is low priority.

### Extension (Chrome)

All storage is in-memory or OPFS — a constant `diskId` of `"browser"` is sufficient. Everything shares one "disk".

## Phased Implementation

### Phase 1: diskId Plumbing (Data Only)

Add `diskId` to `StorageRoot` and populate it from each platform. No behavior change — just data flowing through.

**Changes:**
- `packages/engine/src/storage/types.ts`: Add `diskId?: string` to `StorageRoot`
- `android/.../DownloadRoot.kt`: Add `volumeId` field, extract from SAF URI
- `android/.../EngineController.kt`: Pass `volumeId` as `diskId` in `ContentRoot`
- `desktop/common/src/lib.rs`: Add `disk_id` to `DownloadRoot`
- `desktop/io-daemon/src/files.rs` or `host/src/rpc.rs`: Populate `disk_id` from `st_dev`
- Node.js preset: Populate from `fs.statSync().dev`

**Verification:**
- Unit test: `StorageRootManager` stores and retrieves `diskId`
- Android: Log `diskId` on root add, verify internal vs SD card get different IDs
- Rust: Unit test `get_device_id()` returns same value for two paths on same mount
- Node.js: Integration test with two roots on same filesystem

### Phase 2: DiskIO Skeleton

Introduce `DiskIO` class, one per `diskId`. Initially a thin passthrough — routes to existing queues. `StorageRootManager` creates/manages `DiskIO` instances.

**Changes:**
- New `packages/engine/src/core/disk-io.ts`
- `StorageRootManager`: `diskMap: Map<diskId, DiskIO>`, lifecycle management
- `TorrentContentStorage`: Option to route writes/reads through `DiskIO`

**Verification:**
- All existing tests still pass (passthrough behavior)
- New unit test: `DiskIO` tracks job count and bytes
- Integration: Download + seed still works on all platforms

### Phase 3: Disk Stats & Free Space

`DiskIO` tracks throughput, queue depth, latency. Platforms expose free space.

**Changes:**
- `DiskIO`: Track `bytesWritten`, `bytesRead`, `writeLatencyMs`, `queueDepth`
- Android: `StatFs` on volume path for free space
- Rust: `statvfs` for free space, new endpoint or field on root
- JS: Expose stats via engine diagnostics / debug commands

**Verification:**
- Stats visible in debug output during download
- Free space reported correctly per root
- Disk-full scenario: proactive error before write failure

### Phase 4: Write Buffer Backpressure

`DiskIO` enforces a write buffer watermark. When exceeded, downloading peers pause socket reads.

**Changes:**
- `DiskIO`: `writeBufferBytes`, `highWatermark`, `lowWatermark`
- `DiskIO.asyncWrite()` returns `{ exceeded: boolean }`
- Peer connection: When `exceeded`, stop reading from socket (TCP backpressure propagates naturally)
- When buffer drains below low watermark, resume reading

**Verification:**
- Simulated slow disk (sleep in write handler) triggers backpressure
- Download speed self-regulates instead of OOM
- Fast disk: no backpressure, no throughput regression

### Phase 5: Store Buffer

Recently-written blocks served from memory on read, avoiding disk round-trip.

**Changes:**
- `DiskIO`: `storeBuffer: Map<string, Uint8Array>` keyed by `${torrentId}:${piece}:${offset}`
- `asyncWrite()`: Insert into store buffer before write
- `asyncRead()`: Check store buffer first, fall through to disk on miss
- `asyncHash()`: Check store buffer per block
- Eviction: Remove from store buffer after write completes on disk

**Verification:**
- Upload immediately after download: reads served from buffer (measurable via stats)
- Hash check after write: no disk read needed
- Memory bounded: store buffer size tracked, eviction works

### Phase 6: Per-Torrent Fencing

Metadata operations (delete, move, check) wait for outstanding I/O to complete.

**Changes:**
- `DiskIO`: Per-torrent outstanding op counter
- `asyncDeleteFiles()`: Fence — wait for ops to drain, then delete
- `asyncMoveStorage()`: Fence — wait, then move
- Blocked jobs queued, released when fence drops

**Verification:**
- Delete torrent while downloading: no file-in-use errors
- Move storage while seeding: clean handoff
- No deadlocks under concurrent operations

## Non-Goals

- **mmap**: We cross FFI/HTTP boundaries; mmap's zero-copy benefit is lost at those boundaries
- **Custom allocator**: JS has GC; ActivePieceManager buffer pool handles the hot path
- **Separate hash thread pool**: Hashing is fast in Kotlin/Rust; not worth JS-side separation
- **Cross-disk striping**: RAID-like striping across disks is out of scope
