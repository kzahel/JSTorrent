package com.jstorrent.io.file

import android.content.Context
import android.net.Uri
import android.system.Os
import android.system.OsConstants
import android.util.Log
import androidx.documentfile.provider.DocumentFile
import java.io.FileOutputStream
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.MappedByteBuffer
import java.nio.channels.FileChannel
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

private const val TAG = "FileHandlePool"

/**
 * A pooled file handle that can be reused for multiple writes.
 *
 * For SAF URIs: holds ParcelFileDescriptor + FileChannel
 * For file:// URIs: holds RandomAccessFile or MappedByteBuffer
 */
sealed class PooledHandle : AutoCloseable {
    abstract val lastUsed: Long
    abstract fun updateLastUsed()
    abstract fun writeAt(offset: Long, data: ByteArray)

    /**
     * SAF-based handle using ParcelFileDescriptor.
     * Thread-safe: uses internal lock since channel.position()+write() is not atomic.
     */
    class SafHandle(
        private val pfd: android.os.ParcelFileDescriptor,
        private val channel: FileChannel,
    ) : PooledHandle() {
        private val writeLock = ReentrantLock()

        @Volatile
        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            writeLock.withLock {
                channel.position(offset)
                channel.write(ByteBuffer.wrap(data))
            }
        }

        override fun close() {
            try {
                channel.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing channel", e)
            }
            try {
                pfd.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing PFD", e)
            }
        }
    }

    /**
     * SAF-based memory-mapped handle for pre-allocated files.
     * Uses mmap via the SAF file descriptor for fast random writes.
     * Thread-safe: uses duplicate() to avoid position conflicts.
     */
    class SafMappedHandle(
        private val pfd: android.os.ParcelFileDescriptor,
        private val channel: FileChannel,
        private val buffer: MappedByteBuffer,
        private val fileSize: Long,
    ) : PooledHandle() {
        @Volatile
        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            if (offset + data.size > fileSize) {
                throw IllegalArgumentException("Write beyond file size: offset=$offset, len=${data.size}, fileSize=$fileSize")
            }
            // Use duplicate() to get independent position state for thread safety
            val buf = buffer.duplicate()
            buf.position(offset.toInt())
            buf.put(data)
        }

        override fun close() {
            try {
                buffer.force()
            } catch (e: Exception) {
                Log.w(TAG, "Error forcing SAF mmap buffer", e)
            }
            try {
                channel.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing SAF channel", e)
            }
            try {
                pfd.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing PFD", e)
            }
        }
    }

    /**
     * SAF memory-mapped handle using Os.mmap() directly.
     * This bypasses Java's FileChannel limitations that prevent mmap on SAF file descriptors.
     *
     * Uses JNI via MmapHelper to write directly to mmap'd memory, avoiding Android's
     * hidden API restrictions that block the reflective DirectByteBuffer constructor
     * approach on SDK 35+.
     */
    class SafOsMappedHandle(
        private val pfd: android.os.ParcelFileDescriptor,
        private val address: Long,
        private val fileSize: Long,
    ) : PooledHandle() {

        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            if (offset + data.size > fileSize) {
                throw IllegalArgumentException("Write beyond file size: offset=$offset, len=${data.size}, fileSize=$fileSize")
            }
            // Use JNI to copy data directly to mmap'd memory
            MmapHelper.copyToAddress(address + offset, data, 0, data.size)
        }

        override fun close() {
            try {
                Os.munmap(address, fileSize)
            } catch (e: Exception) {
                Log.w(TAG, "Error unmapping SAF Os.mmap buffer", e)
            }
            try {
                pfd.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing PFD", e)
            }
        }
    }

    /**
     * Multi-region SAF memory-mapped handle for files > 2GB.
     *
     * Uses android.system.Os.mmap() directly to bypass Java's FileChannel limitations.
     * This allows mmap on SAF file descriptors where FileChannel.map() fails due to
     * the channel being read-only or write-only (NonReadable/NonWritableChannelException).
     *
     * Maps fixed-size regions (256MB by default) on-demand with LRU eviction.
     *
     * Thread-safe: uses internal lock for region cache, but actual mmap writes
     * to non-overlapping offsets can proceed in parallel.
     *
     * @param pfd The ParcelFileDescriptor for the SAF file
     * @param fileSize Total file size
     * @param regionSize Size of each mapped region (default 256MB)
     * @param maxRegions Maximum number of regions to keep mapped (LRU cache)
     */
    class SafMultiRegionMappedHandle(
        private val pfd: android.os.ParcelFileDescriptor,
        private val fileSize: Long,
        private val regionSize: Long = MultiRegionMappedHandle.MULTI_REGION_SIZE,
        private val maxRegions: Int = MultiRegionMappedHandle.MAX_MAPPED_REGIONS,
    ) : PooledHandle() {

        private data class MappedRegion(
            val regionIndex: Long,
            val address: Long,  // mmap address from Os.mmap()
            val startOffset: Long,
            val size: Long,
        )

        // LRU cache of mapped regions - protected by regionLock
        private val regions = LinkedHashMap<Long, MappedRegion>(maxRegions, 0.75f, true)
        private val regionLock = ReentrantLock()

        @Volatile
        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            if (offset + data.size > fileSize) {
                throw IllegalArgumentException("Write beyond file size: offset=$offset, len=${data.size}, fileSize=$fileSize")
            }

            val startRegionIdx = offset / regionSize
            val endRegionIdx = (offset + data.size - 1) / regionSize

            if (startRegionIdx == endRegionIdx) {
                // Common case: write fits in single region
                // Get region address under lock, then write outside lock
                val region = getOrMapRegion(startRegionIdx)
                val regionOffset = (offset - region.startOffset).toInt()
                // Write directly to mmap memory - no lock needed for the memcpy
                writeToMmap(region.address + regionOffset, data, 0, data.size)
            } else {
                // Rare case: write spans multiple regions (only at boundaries)
                var dataOffset = 0
                for (regionIdx in startRegionIdx..endRegionIdx) {
                    val region = getOrMapRegion(regionIdx)
                    val regionStart = region.startOffset
                    val regionEnd = regionStart + region.size

                    val writeStart = maxOf(offset + dataOffset, regionStart)
                    val writeEnd = minOf(offset + data.size, regionEnd)
                    val writeLen = (writeEnd - writeStart).toInt()

                    val regionOffset = (writeStart - regionStart).toInt()
                    writeToMmap(region.address + regionOffset, data, dataOffset, writeLen)

                    dataOffset += writeLen
                }
            }
        }

        /**
         * Write data to mmap'd memory region using JNI direct memory copy.
         * This is lock-free - concurrent writes to non-overlapping addresses are safe.
         */
        private fun writeToMmap(address: Long, data: ByteArray, offset: Int, length: Int) {
            MmapHelper.copyToAddress(address, data, offset, length)
        }

        private fun getOrMapRegion(regionIndex: Long): MappedRegion {
            regionLock.withLock {
                // Check cache first (also updates LRU order)
                regions[regionIndex]?.let { return it }

                // Evict oldest regions if at capacity
                while (regions.size >= maxRegions) {
                    val oldest = regions.entries.first()
                    try {
                        Os.munmap(oldest.value.address, oldest.value.size)
                    } catch (e: Exception) {
                        Log.w(TAG, "Error unmapping SAF region ${oldest.key}", e)
                    }
                    regions.remove(oldest.key)
                    Log.d(TAG, "Evicted SAF mmap region ${oldest.key}")
                }

                // Map new region using Os.mmap() directly
                val regionOffset = regionIndex * regionSize
                val regionEnd = minOf(regionOffset + regionSize, fileSize)
                val actualSize = regionEnd - regionOffset

                val address = Os.mmap(
                    0, // let OS choose address
                    actualSize,
                    OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                    OsConstants.MAP_SHARED,
                    pfd.fileDescriptor,
                    regionOffset
                )

                val region = MappedRegion(regionIndex, address, regionOffset, actualSize)
                regions[regionIndex] = region
                Log.d(TAG, "Mapped SAF region $regionIndex via Os.mmap: offset=${regionOffset / (1024 * 1024)}MB, size=${actualSize / (1024 * 1024)}MB, addr=0x${address.toString(16)}")

                return region
            }
        }

        override fun close() {
            regionLock.withLock {
                // Unmap all regions
                for ((idx, region) in regions) {
                    try {
                        Os.munmap(region.address, region.size)
                    } catch (e: Exception) {
                        Log.w(TAG, "Error unmapping SAF region $idx on close", e)
                    }
                }
                regions.clear()
            }
            try {
                pfd.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing PFD", e)
            }
        }

    }

    /**
     * Native file handle using RandomAccessFile.
     * Thread-safe: uses internal lock since raf.seek()+write() is not atomic.
     */
    class NativeHandle(
        private val raf: RandomAccessFile,
    ) : PooledHandle() {
        private val writeLock = ReentrantLock()

        @Volatile
        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            writeLock.withLock {
                raf.seek(offset)
                raf.write(data)
            }
        }

        override fun close() {
            try {
                raf.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing RAF", e)
            }
        }
    }

    /**
     * Memory-mapped file handle for pre-allocated files.
     * Much faster for random writes as it avoids syscall overhead per write.
     * Thread-safe: uses duplicate() to avoid position conflicts.
     */
    class MappedHandle(
        private val raf: RandomAccessFile,
        private val channel: FileChannel,
        private val buffer: MappedByteBuffer,
        private val fileSize: Long,
    ) : PooledHandle() {
        @Volatile
        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            if (offset + data.size > fileSize) {
                throw IllegalArgumentException("Write beyond file size: offset=$offset, len=${data.size}, fileSize=$fileSize")
            }
            // Use duplicate() to get independent position state for thread safety
            // Actual writes to non-overlapping regions are safe (torrent pieces don't overlap)
            val buf = buffer.duplicate()
            buf.position(offset.toInt())
            buf.put(data)
        }

        override fun close() {
            try {
                // Force any remaining writes to disk
                buffer.force()
            } catch (e: Exception) {
                Log.w(TAG, "Error forcing buffer", e)
            }
            try {
                channel.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing channel", e)
            }
            try {
                raf.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing RAF", e)
            }
        }
    }

    /**
     * Multi-region memory-mapped handle for files > 2GB.
     *
     * Maps fixed-size regions (256MB by default) on-demand with LRU eviction.
     * This overcomes the MappedByteBuffer 2GB limit by only mapping portions
     * of the file at a time.
     *
     * Thread-safe: uses internal lock for region cache, but actual mmap writes
     * to non-overlapping offsets can proceed in parallel.
     *
     * @param raf The RandomAccessFile for the underlying file
     * @param channel The FileChannel for mapping regions
     * @param fileSize Total file size
     * @param regionSize Size of each mapped region (default 256MB)
     * @param maxRegions Maximum number of regions to keep mapped (LRU cache)
     */
    class MultiRegionMappedHandle(
        private val raf: RandomAccessFile,
        private val channel: FileChannel,
        private val fileSize: Long,
        private val regionSize: Long = MULTI_REGION_SIZE,
        private val maxRegions: Int = MAX_MAPPED_REGIONS,
    ) : PooledHandle() {

        companion object {
            // 256MB regions - large enough for efficiency, small enough for memory
            const val MULTI_REGION_SIZE = 256L * 1024 * 1024
            // Keep up to 8 regions mapped (2GB total address space usage)
            const val MAX_MAPPED_REGIONS = 8
        }

        private data class MappedRegion(
            val regionIndex: Long,
            val buffer: MappedByteBuffer,
            val startOffset: Long,
            val size: Long,
        )

        // LRU cache of mapped regions - protected by regionLock
        private val regions = LinkedHashMap<Long, MappedRegion>(maxRegions, 0.75f, true)
        private val regionLock = ReentrantLock()

        @Volatile
        override var lastUsed: Long = System.currentTimeMillis()
            private set

        override fun updateLastUsed() {
            lastUsed = System.currentTimeMillis()
        }

        override fun writeAt(offset: Long, data: ByteArray) {
            if (offset + data.size > fileSize) {
                throw IllegalArgumentException("Write beyond file size: offset=$offset, len=${data.size}, fileSize=$fileSize")
            }

            val startRegionIdx = offset / regionSize
            val endRegionIdx = (offset + data.size - 1) / regionSize

            if (startRegionIdx == endRegionIdx) {
                // Common case: write fits in single region
                // Get region under lock, write outside lock
                val region = getOrMapRegion(startRegionIdx)
                val regionOffset = (offset - region.startOffset).toInt()
                // MappedByteBuffer.put() for non-overlapping regions is thread-safe
                // Use duplicate() to avoid position conflicts between threads
                val buf = region.buffer.duplicate()
                buf.position(regionOffset)
                buf.put(data)
            } else {
                // Rare case: write spans multiple regions (only at boundaries)
                var dataOffset = 0
                for (regionIdx in startRegionIdx..endRegionIdx) {
                    val region = getOrMapRegion(regionIdx)
                    val regionStart = region.startOffset
                    val regionEnd = regionStart + region.size

                    val writeStart = maxOf(offset + dataOffset, regionStart)
                    val writeEnd = minOf(offset + data.size, regionEnd)
                    val writeLen = (writeEnd - writeStart).toInt()

                    val regionOffset = (writeStart - regionStart).toInt()
                    val buf = region.buffer.duplicate()
                    buf.position(regionOffset)
                    buf.put(data, dataOffset, writeLen)

                    dataOffset += writeLen
                }
            }
        }

        private fun getOrMapRegion(regionIndex: Long): MappedRegion {
            regionLock.withLock {
                // Check cache first (also updates LRU order)
                regions[regionIndex]?.let { return it }

                // Evict oldest regions if at capacity
                while (regions.size >= maxRegions) {
                    val oldest = regions.entries.first()
                    try {
                        oldest.value.buffer.force()
                    } catch (e: Exception) {
                        Log.w(TAG, "Error forcing region ${oldest.key} before eviction", e)
                    }
                    regions.remove(oldest.key)
                    Log.d(TAG, "Evicted mmap region ${oldest.key}")
                }

                // Map new region
                val regionOffset = regionIndex * regionSize
                val regionEnd = minOf(regionOffset + regionSize, fileSize)
                val actualSize = regionEnd - regionOffset

                val buffer = channel.map(FileChannel.MapMode.READ_WRITE, regionOffset, actualSize)
                val region = MappedRegion(regionIndex, buffer, regionOffset, actualSize)
                regions[regionIndex] = region
                Log.d(TAG, "Mapped region $regionIndex: offset=${regionOffset / (1024 * 1024)}MB, size=${actualSize / (1024 * 1024)}MB")

                return region
            }
        }

        override fun close() {
            regionLock.withLock {
                // Force all mapped regions
                for ((idx, region) in regions) {
                    try {
                        region.buffer.force()
                    } catch (e: Exception) {
                        Log.w(TAG, "Error forcing region $idx on close", e)
                    }
                }
                regions.clear()
            }
            try {
                channel.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing channel", e)
            }
            try {
                raf.close()
            } catch (e: Exception) {
                Log.w(TAG, "Error closing RAF", e)
            }
        }
    }
}

/**
 * LRU pool of open file handles to avoid repeated open/close overhead.
 *
 * SAF file opens have ~10-15ms IPC overhead. By keeping handles open and reusing
 * them, we can amortize this cost across many writes.
 *
 * For files that have been pre-allocated via [preallocate], uses memory-mapped I/O
 * which is significantly faster for random writes (~0.1ms vs 10-15ms).
 *
 * Thread-safe: all operations are synchronized.
 *
 * @param context Android context for SAF operations
 * @param maxHandles Maximum number of open handles (default: 64)
 * @param useMmap Enable memory-mapped I/O for pre-allocated files (default: true)
 */
class FileHandlePool(
    private val context: Context,
    private val maxHandles: Int = 64,
    private val useMmap: Boolean = true,
) {
    /**
     * Cache key that uniquely identifies a file.
     * For SAF: the document URI
     * For native: the absolute file path
     */
    private data class HandleKey(val key: String)

    private val handles = LinkedHashMap<HandleKey, PooledHandle>(16, 0.75f, true)
    private val lock = ReentrantLock()

    // Track pre-allocated file sizes for mmap
    private val preallocatedSizes = mutableMapOf<HandleKey, Long>()

    // Stats for logging
    private var hits = 0
    private var misses = 0
    private var evictions = 0
    private var mmapCount = 0
    private var rafCount = 0
    private var lastStatsLogTime = System.currentTimeMillis()

    companion object {
        // MappedByteBuffer has a 2GB limit (Integer.MAX_VALUE)
        const val MAX_MMAP_SIZE = Int.MAX_VALUE.toLong()
    }

    /**
     * Pre-allocate a SAF file for memory-mapped I/O.
     *
     * Call this before starting to write to enable fast mmap writes.
     * The file must already exist and be opened via SAF.
     *
     * Files <= 2GB use single-region mmap. Files > 2GB use multi-region mmap
     * with 256MB regions and LRU eviction.
     *
     * @param docFile The DocumentFile to pre-allocate
     * @param size The total size to allocate
     * @return true if pre-allocation succeeded, false otherwise
     */
    fun preallocate(docFile: DocumentFile, size: Long): Boolean {
        val key = HandleKey(docFile.uri.toString())

        lock.withLock {
            // Close any existing handle
            handles.remove(key)?.close()

            try {
                // Pre-allocate via SAF - set file length
                context.contentResolver.openFileDescriptor(docFile.uri, "rw")?.use { pfd ->
                    val fos = FileOutputStream(pfd.fileDescriptor)
                    fos.channel.use { channel ->
                        channel.truncate(size)
                        // Force allocation by writing a byte at the end
                        if (size > 0) {
                            channel.position(size - 1)
                            channel.write(ByteBuffer.wrap(byteArrayOf(0)))
                        }
                    }
                } ?: return false

                // Track for mmap - files > 2GB use multi-region mmap
                preallocatedSizes[key] = size
                if (size <= MAX_MMAP_SIZE) {
                    Log.i(TAG, "Pre-allocated SAF ${docFile.name}: ${size / (1024 * 1024)}MB (single-region mmap)")
                } else {
                    Log.i(TAG, "Pre-allocated SAF ${docFile.name}: ${size / (1024 * 1024)}MB (multi-region mmap)")
                }
                return true
            } catch (e: Exception) {
                Log.e(TAG, "SAF pre-allocation failed for ${docFile.name}", e)
                return false
            }
        }
    }

    /**
     * Pre-allocate a native file for memory-mapped I/O.
     *
     * Call this before starting to write to a file to enable fast mmap writes.
     * The file will be created with the specified size, filled with zeros.
     *
     * Files <= 2GB use single-region mmap. Files > 2GB use multi-region mmap
     * with 256MB regions and LRU eviction.
     *
     * @param file The file to pre-allocate
     * @param size The total size to allocate
     * @return true if pre-allocation succeeded, false otherwise
     */
    fun preallocate(file: java.io.File, size: Long): Boolean {
        val key = HandleKey("file://${file.absolutePath}")

        lock.withLock {
            // Close any existing handle
            handles.remove(key)?.close()

            try {
                // Create parent directories
                file.parentFile?.mkdirs()

                // Pre-allocate the file
                RandomAccessFile(file, "rw").use { raf ->
                    raf.setLength(size)
                }

                // Track for mmap - files > 2GB use multi-region mmap
                preallocatedSizes[key] = size
                if (size <= MAX_MMAP_SIZE) {
                    Log.i(TAG, "Pre-allocated ${file.name}: ${size / (1024 * 1024)}MB (single-region mmap)")
                } else {
                    Log.i(TAG, "Pre-allocated ${file.name}: ${size / (1024 * 1024)}MB (multi-region mmap)")
                }
                return true
            } catch (e: Exception) {
                Log.e(TAG, "Pre-allocation failed for ${file.name}", e)
                return false
            }
        }
    }

    /**
     * Check if a file has been pre-allocated.
     */
    fun isPreallocated(file: java.io.File): Boolean {
        val key = HandleKey("file://${file.absolutePath}")
        lock.withLock {
            return preallocatedSizes.containsKey(key)
        }
    }

    /**
     * Write data at the specified offset to a SAF document.
     *
     * If the file has been pre-allocated via [preallocate], uses memory-mapped I/O
     * for significantly faster writes.
     *
     * Thread-safe: global lock is held only for handle map access.
     * Actual writes happen outside the lock for maximum parallelism.
     *
     * @param docFile The DocumentFile to write to
     * @param offset Byte offset to write at
     * @param data Data to write
     * @throws FileManagerException on I/O errors
     */
    fun writeAt(docFile: DocumentFile, offset: Long, data: ByteArray) {
        val key = HandleKey(docFile.uri.toString())

        // Get or create handle under lock, then write outside lock
        val handle = lock.withLock {
            val existing = handles[key]
            if (existing != null) {
                hits++
                existing.updateLastUsed()
                existing
            } else {
                misses++
                logStatsIfNeeded()
                evictIfNeeded()

                val preallocatedSize = preallocatedSizes[key]
                val newHandle = if (useMmap && preallocatedSize != null) {
                    openSafMappedHandle(docFile, preallocatedSize)
                } else {
                    openSafHandle(docFile)
                }
                handles[key] = newHandle
                newHandle
            }
        }

        // Write OUTSIDE the lock - this is the key optimization!
        // Mmap writes to non-overlapping regions are thread-safe.
        try {
            handle.writeAt(offset, data)
        } catch (e: Exception) {
            // Handle went bad, remove from cache
            lock.withLock {
                if (handles[key] === handle) {
                    handles.remove(key)
                    handle.close()
                }
            }
            throw e
        }
    }

    /**
     * Write data at the specified offset to a native file.
     *
     * If the file has been pre-allocated via [preallocate], uses memory-mapped I/O
     * for significantly faster writes.
     *
     * Thread-safe: global lock is held only for handle map access.
     * Actual writes happen outside the lock for maximum parallelism.
     *
     * @param file The File to write to
     * @param offset Byte offset to write at
     * @param data Data to write
     * @throws FileManagerException on I/O errors
     */
    fun writeAt(file: java.io.File, offset: Long, data: ByteArray) {
        val key = HandleKey("file://${file.absolutePath}")

        // Get or create handle under lock, then write outside lock
        val handle = lock.withLock {
            val existing = handles[key]
            if (existing != null) {
                hits++
                existing.updateLastUsed()
                existing
            } else {
                misses++
                logStatsIfNeeded()
                evictIfNeeded()

                // Create parent directories if needed
                file.parentFile?.mkdirs()

                val preallocatedSize = preallocatedSizes[key]
                val newHandle = if (useMmap && preallocatedSize != null) {
                    openMappedHandle(file, preallocatedSize)
                } else {
                    openNativeHandle(file)
                }
                handles[key] = newHandle
                newHandle
            }
        }

        // Write OUTSIDE the lock - this is the key optimization!
        // Mmap writes to non-overlapping regions are thread-safe.
        try {
            handle.writeAt(offset, data)
        } catch (e: Exception) {
            lock.withLock {
                if (handles[key] === handle) {
                    handles.remove(key)
                    handle.close()
                }
            }
            throw e
        }
    }

    /**
     * Log stats periodically (every 5 seconds).
     * Must be called while holding lock.
     */
    private fun logStatsIfNeeded() {
        val now = System.currentTimeMillis()
        if (now - lastStatsLogTime >= 5000 && (hits + misses) > 0) {
            val hitRate = hits * 100.0 / (hits + misses)
            Log.i(TAG, "${handles.size}/$maxHandles handles, hits=$hits, misses=$misses (%.1f%% hit rate), evictions=$evictions, mmap=$mmapCount, raf=$rafCount".format(hitRate))
            lastStatsLogTime = now
        }
    }

    /**
     * Invalidate a specific handle (e.g., after a write error).
     */
    fun invalidate(uri: Uri) {
        val key = HandleKey(uri.toString())
        lock.withLock {
            handles.remove(key)?.close()
        }
    }

    /**
     * Invalidate a native file handle.
     */
    fun invalidate(file: java.io.File) {
        val key = HandleKey("file://${file.absolutePath}")
        lock.withLock {
            handles.remove(key)?.close()
        }
    }

    /**
     * Close all handles and clear the pool.
     */
    fun closeAll() {
        lock.withLock {
            Log.i(TAG, "Closing ${handles.size} handles (hits=$hits, misses=$misses, evictions=$evictions)")
            handles.values.forEach { it.close() }
            handles.clear()
            hits = 0
            misses = 0
            evictions = 0
        }
    }

    /**
     * Get pool statistics for debugging.
     */
    fun getStats(): String {
        lock.withLock {
            val hitRate = if (hits + misses > 0) {
                (hits * 100.0 / (hits + misses))
            } else {
                0.0
            }
            return "FileHandlePool: ${handles.size}/$maxHandles handles, " +
                    "hits=$hits, misses=$misses (${String.format("%.1f", hitRate)}%), " +
                    "evictions=$evictions"
        }
    }

    /**
     * Evict oldest handle if at capacity.
     * Must be called while holding lock.
     */
    private fun evictIfNeeded() {
        while (handles.size >= maxHandles) {
            val oldest = handles.entries.firstOrNull() ?: break
            Log.d(TAG, "Evicting handle: ${oldest.key}")
            oldest.value.close()
            handles.remove(oldest.key)
            evictions++
        }
    }

    /**
     * Open a SAF handle for the given DocumentFile.
     */
    private fun openSafHandle(docFile: DocumentFile): PooledHandle.SafHandle {
        rafCount++
        val pfd = context.contentResolver.openFileDescriptor(docFile.uri, "rw")
            ?: throw FileManagerException.CannotOpenFile(docFile.uri.toString())

        val fos = FileOutputStream(pfd.fileDescriptor)
        val channel = fos.channel

        return PooledHandle.SafHandle(pfd, channel)
    }

    /**
     * Open a memory-mapped SAF handle. Uses multi-region mmap for files > 2GB.
     */
    private fun openSafMappedHandle(docFile: DocumentFile, size: Long): PooledHandle {
        mmapCount++
        val pfd = context.contentResolver.openFileDescriptor(docFile.uri, "rw")
            ?: throw FileManagerException.CannotOpenFile(docFile.uri.toString())

        // Use Os.mmap() directly for SAF files.
        // Java's FileChannel.map() fails on SAF because:
        // - FileInputStream gives read-only channel (NonWritableChannelException)
        // - FileOutputStream gives write-only channel (NonReadableChannelException)
        // Os.mmap() bypasses this by going directly to the OS mmap syscall.

        return if (size <= MAX_MMAP_SIZE) {
            // Single region - use Os.mmap for the whole file
            val address = Os.mmap(
                0,
                size,
                OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                OsConstants.MAP_SHARED,
                pfd.fileDescriptor,
                0
            )
            Log.d(TAG, "Opened SAF mmap handle for ${docFile.name}: ${size / (1024 * 1024)}MB (single-region via Os.mmap, addr=0x${address.toString(16)})")
            PooledHandle.SafOsMappedHandle(pfd, address, size)
        } else {
            // Multi-region for large files
            Log.d(TAG, "Opened SAF multi-region mmap handle for ${docFile.name}: ${size / (1024 * 1024)}MB")
            PooledHandle.SafMultiRegionMappedHandle(pfd, size)
        }
    }

    /**
     * Open a native handle for the given File.
     */
    private fun openNativeHandle(file: java.io.File): PooledHandle.NativeHandle {
        rafCount++
        val raf = RandomAccessFile(file, "rw")
        return PooledHandle.NativeHandle(raf)
    }

    /**
     * Open a memory-mapped native handle. Uses multi-region mmap for files > 2GB.
     *
     * Memory-mapped I/O is significantly faster for random writes because:
     * - Writes go directly to the page cache, no syscall per write
     * - The kernel handles flushing to disk asynchronously
     * - No per-write overhead from file descriptor operations
     */
    private fun openMappedHandle(file: java.io.File, size: Long): PooledHandle {
        mmapCount++
        val raf = RandomAccessFile(file, "rw")
        val channel = raf.channel

        return if (size <= MAX_MMAP_SIZE) {
            // Single region - map entire file
            val buffer = channel.map(FileChannel.MapMode.READ_WRITE, 0, size)
            Log.d(TAG, "Opened mmap handle for ${file.name}: ${size / (1024 * 1024)}MB (single-region)")
            PooledHandle.MappedHandle(raf, channel, buffer, size)
        } else {
            // Multi-region for large files
            Log.d(TAG, "Opened multi-region mmap handle for ${file.name}: ${size / (1024 * 1024)}MB")
            PooledHandle.MultiRegionMappedHandle(raf, channel, size)
        }
    }
}
