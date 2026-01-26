# Mmap Disk Write Experiment - Findings

**Date:** January 2026
**Status:** Experiment concluded - not moving forward
**Branch:** `experiment/mmap-disk-writes`

## Goal

Reduce per-write latency from ~10ms to ~0.1ms by using memory-mapped I/O instead of traditional seek/write syscalls.

## What Was Built

### New Components (~1,200 lines)

1. **FileHandlePool.kt** (956 lines)
   - LRU cache of open file handles (avoids SAF's ~10-15ms open overhead)
   - Support for both SAF (content://) and native (file://) URIs
   - Memory-mapped handle types:
     - `SafOsMappedHandle` - Single-region mmap via `Os.mmap()` for files <= 2GB
     - `SafMultiRegionMappedHandle` - Multi-region with 256MB chunks for files > 2GB
     - Similar native variants for file:// URIs

2. **MmapHelper.kt** + **mmap-helper.c** (227 lines)
   - JNI wrapper for direct memory operations
   - `copyToAddress()` - memcpy from Java byte[] to mmap'd address
   - Bypasses Android's hidden API restrictions on DirectByteBuffer

3. **Pre-allocation Support**
   - `FileManager.preallocate()` - truncates file to final size before download
   - `TorrentContentStorage` calls preallocate on torrent start
   - JS binding: `__jstorrent_file_preallocate`

4. **Detailed Timing Instrumentation**
   - Breakdown: dispatch, hash, write, post times
   - Logged every 5 seconds with throughput stats

## Results on Pixel 7a

### Expected
- Write latency: ~0.1ms (pure memcpy to mmap'd memory)
- Throughput: Limited only by storage bandwidth

### Actual
| Metric | Value |
|--------|-------|
| Write latency | **10-15ms** (no improvement) |
| Throughput | 35-40 MB/s (good, due to async parallelism) |
| Hash time | 3-4ms |
| Dispatch overhead | 0.5ms |

### Timing Breakdown (typical)
```
Verified write: 40.00 MB/s, 201 writes, pending=0 (max=2)
  Timing breakdown: total=14ms, dispatch=0.5ms, hash=3.1ms, write=11.3ms, post=0.1ms
```

## Why It Didn't Work

The **write=11ms** is the mmap memcpy, which should be sub-millisecond. The culprit is **page fault overhead**:

1. **Pre-allocation doesn't pre-fault pages**
   - `truncate()` sets file size but doesn't allocate physical pages
   - Pages are allocated lazily on first write (minor page fault per 4KB page)
   - A 1MB piece touches ~256 pages = ~256 page faults

2. **Page fault cost**
   - Each fault: kernel allocates page, updates page tables, may need to read from disk
   - Even "fast" minor faults add up: 256 faults * ~40μs = ~10ms

3. **Potential dirty page writeback pressure**
   - Kernel limits dirty pages in memory
   - When limit reached, writes stall until pages are flushed

## Alternatives Not Tried

1. **MAP_POPULATE** - Pre-faults all pages at mmap time
   - Pro: Eliminates page faults during writes
   - Con: Blocks for entire file at startup (bad for large files)

2. **madvise(MADV_WILLNEED)** - Async pre-fault hint
   - Pro: Non-blocking, kernel pre-faults in background
   - Con: Race condition if writes happen before pre-fault completes

3. **fallocate() with FALLOC_FL_ZERO_RANGE**
   - Pro: Actually allocates blocks on filesystem
   - Con: Not available on all Android filesystems

## Conclusion

The mmap approach adds significant complexity (~1,200 lines of Kotlin/JNI) but doesn't improve write latency because page faults dominate. The throughput is still good (35-40 MB/s) due to async parallelism in the write pipeline.

**What's worth keeping:**
- `FileHandlePool` concept (caching open handles is useful for SAF)
- Async verified writes with coroutines
- Timing instrumentation for debugging

**What's not worth the complexity:**
- JNI mmap helper
- Os.mmap() for SAF files
- Multi-region mmap logic
- Pre-allocation (doesn't help without pre-faulting)

A simpler approach using `FileChannel.write()` with cached handles would likely perform identically with much less code.

## Files in This Branch

### New Files
- `android/io-core/src/main/java/com/jstorrent/io/file/FileHandlePool.kt`
- `android/io-core/src/main/java/com/jstorrent/io/file/MmapHelper.kt`
- `android/io-core/src/main/cpp/CMakeLists.txt`
- `android/io-core/src/main/cpp/mmap-helper.c`
- `android/io-core/src/androidTest/.../MmapHelperTest.kt`
- `android/io-core/src/androidTest/.../ThroughputBenchmarkTest.kt`

### Modified Files
- `android/io-core/build.gradle.kts` - NDK/CMake config
- `android/io-core/.../FileManager.kt` - preallocate interface
- `android/io-core/.../FileManagerImpl.kt` - FileHandlePool integration
- `android/quickjs-engine/.../FileBindings.kt` - async writes, timing
- `android/quickjs-engine/.../EngineController.kt` - wiring
- `packages/engine/src/core/torrent-content-storage.ts` - preallocate calls
- `packages/engine/src/adapters/native/*` - JS bindings
