# ChromeOS SAF "Not a Descendant" Error

## Summary

On ChromeOS (ARCVM), batch piece writes from the extension to the Android companion intermittently fail with:

```
SecurityException: Document primary:Download/JSTorrent/Big Buck Bunny/Big Buck Bunny.mp4
is not a descendant of primary:Download/JSTorrent
```

This error occurs despite the document ID clearly starting with the tree document ID.

## Environment

- ChromeOS with Android Runtime (ARCVM)
- Extension + Android companion mode
- SAF (Storage Access Framework) tree URI for downloads

## Symptoms

- Intermittent write failures (some writes succeed, some fail)
- Error originates from `DocumentsProvider.enforceTree()` in Android framework
- Affects files in **subdirectories** (e.g., `Big Buck Bunny/Big Buck Bunny.mp4`)
- The retry mechanism eventually recovers, but causes slowdowns

## Key Observation

**Standalone mode appears more stable.** The same file operations succeed more reliably when running the engine natively on Android (standalone app). Investigation reveals both modes use the same SAF code path, but differ in write timing patterns.

## Root Cause

Both standalone and companion modes use **the same SAF write code** (`FileManagerImpl.write()`) which opens a **fresh ParcelFileDescriptor for every single write**:

```kotlin
// FileManagerImpl.kt:197 - No pooling for SAF!
context.contentResolver.openFileDescriptor(file!!.uri, "rw")?.use { pfd ->
    FileOutputStream(pfd.fileDescriptor).use { fos ->
        val channel = fos.channel
        channel.position(offset)
        channel.write(ByteBuffer.wrap(data))
    }
}
```

The native `file://` path has a pooled file handle system (`PooledFileHandle`), but SAF `content://` URIs don't benefit from this.

**Why companion mode fails more:**
- **Companion**: 6 concurrent HTTP connections with sustained write pressure, no gaps
- **Standalone**: Writes batched at JS tick boundary (~16ms), creating natural pauses

The rapid concurrent `openFileDescriptor()` calls from companion mode appear to corrupt ARCVM's SAF state.

## Technical Details

### URIs Involved

**Tree URI (granted permission):**
```
content://com.android.externalstorage.documents/tree/primary%3ADownload%2FJSTorrent
```
Decoded tree document ID: `primary:Download/JSTorrent`

**File URI (being accessed):**
```
content://com.android.externalstorage.documents/tree/primary%3ADownload%2FJSTorrent/document/primary%3ADownload%2FJSTorrent%2FBig%20Buck%20Bunny%2FBig%20Buck%20Bunny.mp4
```
Decoded document ID: `primary:Download/JSTorrent/Big Buck Bunny/Big Buck Bunny.mp4`

### The Failing Check

Android's `DocumentsProvider.enforceTree()` checks:
```java
documentId.equals(treeDocumentId) || documentId.startsWith(treeDocumentId + "/")
```

The document ID `primary:Download/JSTorrent/Big Buck Bunny/...` clearly starts with `primary:Download/JSTorrent/`, so this check **should pass** but doesn't.

### Possible Case Mismatch

Observed discrepancy between SAF and filesystem:
- SAF tree: `primary:Download/JSTorrent` (mixed case)
- Filesystem: `/storage/emulated/0/Download/jstorrent` (lowercase)

This may or may not be related to the issue.

## Differences: Companion vs Standalone Mode

| Aspect | Companion Mode | Standalone Mode |
|--------|---------------|-----------------|
| Write source | HTTP batch writes from extension | Direct engine writes via FFI |
| Concurrency | 6 parallel WriteWorker threads | Dispatchers.IO (up to 64 threads) |
| Batching | Continuous stream from 6 HTTP connections | Batched at tick boundary (~16ms) |
| Write pattern | Sustained pressure, no gaps | Bursty with natural pauses between ticks |
| File creation | Created on first write via SAF | Same |
| File access | Fresh ParcelFileDescriptor per write | **Same - no pooling for SAF URIs** |

**Critical finding:** Both modes use the same code path for SAF writes in `FileManagerImpl.write()`:
```kotlin
// Line 197 - opens fresh PFD for EVERY write
context.contentResolver.openFileDescriptor(file!!.uri, "rw")?.use { pfd ->
    FileOutputStream(pfd.fileDescriptor).use { fos ->
        val channel = fos.channel
        channel.position(offset)
        channel.write(ByteBuffer.wrap(data))
    }
}
```

The native file handle pool (`PooledFileHandle` with `FileChannel`) is only used for `file://` URIs (app-private "default" root), NOT for SAF `content://` URIs.

## Hypotheses

### 1. Concurrent `openFileDescriptor()` Race Condition (Most Likely)
Both companion (6 threads) and standalone (Dispatchers.IO) run concurrent writes, but:
- **Companion**: Sustained pressure from 6 HTTP connections with no natural gaps
- **Standalone**: Batched at tick boundary (~16ms), creating natural pauses

Rapid concurrent `openFileDescriptor()` calls on the same file may corrupt ARCVM's SAF state.

### 2. DocumentFile Cache Staleness
The DocumentFile cache may return stale references that have invalid URIs after concurrent operations. The `exists()` check on cached entries also triggers the same error.

### 3. SAF Permission State Corruption
Rapid concurrent `openFileDescriptor()` calls might corrupt the permission state in ChromeOS's ARCVM SAF implementation.

### 4. File Handle Exhaustion
Opening many file descriptors rapidly might cause the SAF to enter an error state.

## Attempted Fixes (Did Not Work)

1. **Using `DocumentsContract.buildDocumentUriUsingTree()`** - Manually constructing the URI produces the same result
2. **Building document ID from tree ID + path** - Same error
3. **Catching `exists()` exceptions** - The actual `openFileDescriptor()` still fails

## Proposed Solutions

### 1. Add SAF File Descriptor Pooling (Recommended)
Add `PooledSafHandle` similar to `PooledFileHandle` for native files:

```kotlin
private class PooledSafHandle(
    val uri: Uri,
    val pfd: ParcelFileDescriptor,
    @Volatile var lastAccessTime: Long = System.currentTimeMillis()
) {
    val channel: FileChannel = FileOutputStream(pfd.fileDescriptor).channel

    fun writeAt(offset: Long, data: ByteArray) {
        lastAccessTime = System.currentTimeMillis()
        val buffer = ByteBuffer.wrap(data)
        var written = 0
        while (buffer.hasRemaining()) {
            written += channel.write(buffer, offset + written)
        }
    }

    fun close() {
        channel.close()
        pfd.close()
    }
}
```

Key changes in `FileManagerImpl`:
- Add `safHandlePool: LinkedHashMap<String, PooledSafHandle>` (keyed by `"$rootUri|$path"`)
- `write()` for SAF: Get or create pooled handle, use `writeAt()` for positioned I/O
- LRU eviction after idle timeout (same as native pool)
- `closeAllHandles()` closes both native and SAF pools

This mirrors how native file:// writes already work with `PooledFileHandle`.

### 2. Reduce Write Concurrency (Quick Fix)
Limit WriteWorkerPool to fewer threads (e.g., 2 instead of 6) to reduce concurrent SAF access.

```kotlin
// WriteWorkerPool.kt
class WriteWorkerPool(
    private val fileManager: FileManager,
    private val workerCount: Int = 2,  // Changed from 6
    ...
)
```

### 3. Per-File Write Serialization
Add per-file locks to ensure only one thread writes to each file at a time:

```kotlin
private val fileLocks = ConcurrentHashMap<String, ReentrantLock>()

override fun write(rootUri: Uri, relativePath: String, offset: Long, data: ByteArray) {
    val lockKey = "$rootUri|$relativePath"
    val lock = fileLocks.computeIfAbsent(lockKey) { ReentrantLock() }
    lock.withLock {
        // ... existing write code
    }
}
```

This would serialize writes to the same file while allowing parallel writes to different files.

### 4. Flatten Directory Structure
Avoid subdirectories by placing files directly in the root download folder (would require torrent structure changes).

### 5. Re-grant SAF Permission
Have users re-select the download folder to get a fresh tree URI with correct metadata.

### 6. Use Direct File I/O
On ChromeOS, request `MANAGE_EXTERNAL_STORAGE` permission to bypass SAF entirely.

## Files Involved

- `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt` - SAF file operations
- `android/companion-server/src/main/java/com/jstorrent/companion/server/streaming/WriteWorkerPool.kt` - Concurrent write workers
- `packages/engine/src/adapters/daemon/daemon-file-handle.ts` - Extension-side batch write handling

## Next Steps

1. **Quick test**: Reduce WriteWorkerPool to 2 threads and verify if errors decrease
2. **Implement SAF FD pooling**: Add `PooledSafHandle` to `FileManagerImpl` mirroring native pool
3. **Test on ChromeOS**: Verify pooled SAF handles work correctly in ARCVM
4. **Consider per-file serialization**: If pooling is complex, per-file locks may be simpler

## References

- Android SAF documentation: https://developer.android.com/guide/topics/providers/document-provider
- ARCVM known issues: (need to research)
