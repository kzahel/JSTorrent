# SAF File Descriptor Pooling Implementation Plan

## Goal

Add pooled `ParcelFileDescriptor` handling for SAF writes in `FileManagerImpl`, mirroring the existing `PooledFileHandle` mechanism for native `file://` URIs. This will reduce concurrent `openFileDescriptor()` calls that cause transient "not a descendant" errors on ChromeOS ARCVM.

## Background

Currently:
- **Native `file://` writes**: Use `PooledFileHandle` with `FileChannel.write(buffer, position)` - pooled, thread-safe
- **SAF `content://` writes**: Open fresh `ParcelFileDescriptor` for every write - not pooled, causes race conditions

## Implementation

### Phase 1: Add PooledSafHandle Class

**File**: `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt`

Add new private class after `PooledFileHandle`:

```kotlin
/**
 * Pooled SAF file handle using FileChannel for lock-free positioned I/O.
 * Similar to PooledFileHandle but for SAF content:// URIs.
 */
private class PooledSafHandle(
    val cacheKey: String,  // "$rootUri|$relativePath"
    val pfd: ParcelFileDescriptor,
    @Volatile var lastAccessTime: Long = System.currentTimeMillis()
) {
    // Use FileOutputStream to get FileChannel from ParcelFileDescriptor
    private val fos = FileOutputStream(pfd.fileDescriptor)
    val channel: FileChannel = fos.channel

    /**
     * Write data at the given position without seeking.
     * Uses FileChannel.write(buffer, position) which is atomic and thread-safe.
     */
    fun writeAt(offset: Long, data: ByteArray) {
        lastAccessTime = System.currentTimeMillis()
        val buffer = ByteBuffer.wrap(data)
        var written = 0
        while (buffer.hasRemaining()) {
            written += channel.write(buffer, offset + written)
        }
    }

    /**
     * Read data from the given position without seeking.
     */
    fun readAt(offset: Long, length: Int): ByteArray {
        lastAccessTime = System.currentTimeMillis()
        val buffer = ByteBuffer.allocate(length)
        var totalRead = 0
        while (buffer.hasRemaining()) {
            val read = channel.read(buffer, offset + totalRead)
            if (read == -1) break
            totalRead += read
        }
        if (totalRead < length) {
            throw IllegalStateException("Could not read $length bytes, only got $totalRead")
        }
        buffer.flip()
        return buffer.array()
    }

    fun close() {
        try {
            channel.close()
            fos.close()
            pfd.close()
        } catch (e: Exception) {
            Log.w(TAG, "Error closing SAF handle: $cacheKey", e)
        }
    }
}
```

### Phase 2: Add SAF Handle Pool Infrastructure

Add to `FileManagerImpl` class properties (after `fileHandlePool`):

```kotlin
/**
 * Pool of open SAF file handles.
 * Key: "$rootUri|$relativePath"
 */
private val safHandlePool = LinkedHashMap<String, PooledSafHandle>(maxFileHandles, 0.75f, true)
private val safHandleLock = ReentrantLock()
```

### Phase 3: Add getPooledSafHandle Method

Add new method to get or create pooled SAF handle:

```kotlin
/**
 * Get or create a pooled SAF file handle.
 * Creates file and parent directories if needed.
 */
private fun getPooledSafHandle(rootUri: Uri, relativePath: String): PooledSafHandle {
    val cacheKey = "$rootUri|$relativePath"

    safHandleLock.withLock {
        // Check if already in pool
        safHandlePool[cacheKey]?.let { return it }

        // Evict idle handles if pool is full
        maybeEvictSafHandles()

        // Get or create the DocumentFile
        var file = getCachedFile(rootUri, relativePath)
        if (file == null) {
            // Use per-path lock to prevent race during file creation
            val lock = creationLocks.computeIfAbsent(cacheKey) { ReentrantLock() }
            lock.withLock {
                file = getCachedFile(rootUri, relativePath)
                if (file == null) {
                    file = createFile(rootUri, relativePath)
                        ?: throw FileManagerException.CannotCreateFile(relativePath)
                    cacheFile(rootUri, relativePath, file)
                }
            }
            if (!lock.hasQueuedThreads()) {
                creationLocks.remove(cacheKey, lock)
            }
        }

        // Open ParcelFileDescriptor in read-write mode
        val pfd = context.contentResolver.openFileDescriptor(file!!.uri, "rw")
            ?: throw FileManagerException.CannotOpenFile(relativePath)

        val handle = PooledSafHandle(cacheKey, pfd)
        safHandlePool[cacheKey] = handle
        return handle
    }
}
```

### Phase 4: Add SAF Handle Eviction

Add eviction method for SAF handles:

```kotlin
/**
 * Evict SAF handles that haven't been used recently or if pool is too large.
 * Must be called with safHandleLock held.
 */
private fun maybeEvictSafHandles() {
    val now = System.currentTimeMillis()

    val toEvict = mutableListOf<String>()

    for ((key, handle) in safHandlePool) {
        if (now - handle.lastAccessTime > handleIdleTimeoutMs) {
            toEvict.add(key)
        }
    }

    // Also evict oldest if over capacity
    while (safHandlePool.size - toEvict.size >= maxFileHandles) {
        val oldest = safHandlePool.entries.firstOrNull { it.key !in toEvict }
        if (oldest != null) {
            toEvict.add(oldest.key)
        } else {
            break
        }
    }

    for (key in toEvict) {
        safHandlePool.remove(key)?.close()
    }

    if (toEvict.isNotEmpty()) {
        Log.d(TAG, "Evicted ${toEvict.size} SAF handles, pool size: ${safHandlePool.size}")
    }
}
```

### Phase 5: Update write() Method

Replace the SAF write implementation in `write()`:

**Before** (lines 171-222):
```kotlin
override fun write(rootUri: Uri, relativePath: String, offset: Long, data: ByteArray) {
    if (isFileUri(rootUri)) {
        return writeNative(rootUri, relativePath, offset, data)
    }

    try {
        // ... file creation logic ...

        // Use ParcelFileDescriptor for true random access writes
        context.contentResolver.openFileDescriptor(file!!.uri, "rw")?.use { pfd ->
            FileOutputStream(pfd.fileDescriptor).use { fos ->
                val channel = fos.channel
                channel.position(offset)
                channel.write(ByteBuffer.wrap(data))
            }
        } ?: throw FileManagerException.CannotOpenFile(relativePath)
    } catch ...
}
```

**After**:
```kotlin
override fun write(rootUri: Uri, relativePath: String, offset: Long, data: ByteArray) {
    if (isFileUri(rootUri)) {
        return writeNative(rootUri, relativePath, offset, data)
    }

    try {
        val handle = getPooledSafHandle(rootUri, relativePath)
        handle.writeAt(offset, data)
    } catch (e: FileManagerException) {
        throw e
    } catch (e: Exception) {
        Log.e(TAG, "Error writing file: ${e.message}", e)
        val msg = e.message ?: ""
        when {
            msg.contains("ENOSPC") || msg.contains("No space") -> {
                throw FileManagerException.DiskFull(relativePath)
            }
            msg.contains("EACCES") || msg.contains("EPERM") ||
                    msg.contains("Permission denied") -> {
                throw FileManagerException.PermissionDenied(relativePath)
            }
            else -> {
                throw FileManagerException.WriteError(relativePath, e)
            }
        }
    }
}
```

### Phase 6: Update read() Method (Optional but Recommended)

Similarly update `read()` to use pooled handles for SAF:

```kotlin
override fun read(rootUri: Uri, relativePath: String, offset: Long, length: Int): ByteArray {
    if (isFileUri(rootUri)) {
        return readNative(rootUri, relativePath, offset, length)
    }

    try {
        val handle = getPooledSafHandle(rootUri, relativePath)
        return handle.readAt(offset, length)
    } catch (e: FileManagerException) {
        throw e
    } catch (e: IllegalStateException) {
        throw FileManagerException.InsufficientData(relativePath, length, 0)
    } catch (e: Exception) {
        Log.e(TAG, "Error reading file: ${e.message}", e)
        throw FileManagerException.ReadError(relativePath, e)
    }
}
```

### Phase 7: Update closeAllHandles()

Update to close both native and SAF pools:

```kotlin
/**
 * Close all pooled file handles (both native and SAF).
 */
fun closeAllHandles() {
    fileHandleLock.withLock {
        for ((_, handle) in fileHandlePool) {
            handle.close()
        }
        fileHandlePool.clear()
    }

    safHandleLock.withLock {
        for ((_, handle) in safHandlePool) {
            handle.close()
        }
        safHandlePool.clear()
    }

    Log.d(TAG, "Closed all file handles")
}
```

### Phase 8: Handle Pool Invalidation on Delete

Update `delete()` to invalidate pooled handles:

```kotlin
override fun delete(rootUri: Uri, relativePath: String): Boolean {
    if (isFileUri(rootUri)) {
        return deleteNative(rootUri, relativePath)
    }

    // Close any pooled handle for this file
    val cacheKey = "$rootUri|$relativePath"
    safHandleLock.withLock {
        safHandlePool.remove(cacheKey)?.close()
    }

    val doc = resolvePath(rootUri, relativePath) ?: return false
    val deleted = doc.delete()
    if (deleted) {
        // Invalidate cache entries for this path and descendants
        val cachePrefix = "$rootUri|$relativePath"
        synchronized(cacheLock) {
            documentFileCache.keys.removeAll { it.startsWith(cachePrefix) }
        }
        // Also close any handles for descendants
        safHandleLock.withLock {
            val toClose = safHandlePool.keys.filter { it.startsWith(cachePrefix) }
            for (key in toClose) {
                safHandlePool.remove(key)?.close()
            }
        }
    }
    return deleted
}
```

## Testing

### Unit Tests

Add to `FileManagerImplTest.kt`:

1. **testSafHandlePooling**: Verify same handle is reused for multiple writes
2. **testSafHandleEviction**: Verify handles are evicted after idle timeout
3. **testSafHandlePoolCapacity**: Verify pool doesn't exceed max size
4. **testConcurrentSafWrites**: Verify thread safety with parallel writes

### Integration Tests

1. Test on ChromeOS ARCVM with companion mode
2. Verify no "not a descendant" errors with 6 concurrent writers
3. Benchmark write throughput before/after

### Manual Testing

1. Download large torrent on ChromeOS with companion mode
2. Monitor logcat for SAF-related errors
3. Compare error rates with old implementation

## Rollout

1. **Phase 1**: Implement and test locally
2. **Phase 2**: Deploy to ChromeOS test device, verify stability
3. **Phase 3**: Monitor for regressions (memory usage, handle leaks)

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Handle leaks | LRU eviction + `closeAllHandles()` on shutdown |
| Memory pressure | Limit pool size (default 32), short idle timeout (30s) |
| Stale handles after file deletion | Invalidate handles in `delete()` |
| Thread contention | Lock per operation is brief; FileChannel I/O is lock-free |

## Success Criteria

- Zero "not a descendant" errors during ChromeOS companion download
- Write throughput maintained or improved
- No memory leaks (handle count stable over time)

## Files to Modify

1. `android/io-core/src/main/java/com/jstorrent/io/file/FileManagerImpl.kt` - Main implementation
2. `android/io-core/src/test/java/com/jstorrent/io/file/FileManagerImplTest.kt` - Unit tests

## Estimated Effort

- Implementation: ~2-3 hours
- Testing: ~1-2 hours
- Total: ~4-5 hours
