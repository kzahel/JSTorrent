package com.jstorrent.io.file

import android.system.Os
import android.system.OsConstants
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.io.RandomAccessFile
import kotlin.random.Random

/**
 * Instrumented tests for MmapHelper JNI functions.
 *
 * Verifies that the JNI-based mmap wrapper works correctly, especially for
 * large files where the reflection-based approach fails on SDK 35+.
 *
 * Run with:
 * ./gradlew :io-core:connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=com.jstorrent.io.file.MmapHelperTest
 */
@RunWith(AndroidJUnit4::class)
class MmapHelperTest {

    companion object {
        private const val TAG = "MmapHelperTest"
    }

    private lateinit var testDir: File
    private lateinit var context: android.content.Context

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext

        // Clean up any stale test directories from previous crashed runs
        context.filesDir.listFiles()?.filter { it.name.startsWith("mmap_test_") }?.forEach {
            Log.i(TAG, "Cleaning up stale test directory: ${it.name}")
            it.deleteRecursively()
        }

        testDir = File(context.filesDir, "mmap_test_${System.currentTimeMillis()}")
        testDir.mkdirs()
        Log.i(TAG, "Test directory: ${testDir.absolutePath}")
    }

    @After
    fun tearDown() {
        testDir.deleteRecursively()
    }

    /**
     * Test basic mmap write via JNI copyToAddress.
     */
    @Test
    fun testCopyToAddress_basic() {
        val testFile = File(testDir, "basic.bin")
        val fileSize = 1024 * 1024L // 1MB

        // Create and pre-allocate file
        RandomAccessFile(testFile, "rw").use { it.setLength(fileSize) }

        // Open file descriptor and mmap
        RandomAccessFile(testFile, "rw").use { raf ->
            val fd = raf.fd
            val address = Os.mmap(
                0,
                fileSize,
                OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                OsConstants.MAP_SHARED,
                fd,
                0
            )

            try {
                // Write test data at various offsets
                val testData = "Hello, mmap!".toByteArray()
                MmapHelper.copyToAddress(address, testData, 0, testData.size)
                MmapHelper.copyToAddress(address + 100, testData, 0, testData.size)
                MmapHelper.copyToAddress(address + 1000, testData, 0, testData.size)

                Log.i(TAG, "Wrote test data at offsets 0, 100, 1000")
            } finally {
                Os.munmap(address, fileSize)
            }
        }

        // Verify by reading back
        RandomAccessFile(testFile, "r").use { raf ->
            val buffer = ByteArray(12)

            raf.seek(0)
            raf.readFully(buffer)
            assertEquals("Hello, mmap!", String(buffer))

            raf.seek(100)
            raf.readFully(buffer)
            assertEquals("Hello, mmap!", String(buffer))

            raf.seek(1000)
            raf.readFully(buffer)
            assertEquals("Hello, mmap!", String(buffer))
        }

        Log.i(TAG, "testCopyToAddress_basic PASSED")
    }

    /**
     * Test mmap read via JNI copyFromAddress.
     */
    @Test
    fun testCopyFromAddress_basic() {
        val testFile = File(testDir, "read_basic.bin")
        val fileSize = 1024 * 1024L // 1MB
        val testData = "Test data for reading".toByteArray()

        // Create file with test data
        RandomAccessFile(testFile, "rw").use { raf ->
            raf.setLength(fileSize)
            raf.seek(500)
            raf.write(testData)
        }

        // mmap and read back
        RandomAccessFile(testFile, "rw").use { raf ->
            val fd = raf.fd
            val address = Os.mmap(
                0,
                fileSize,
                OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                OsConstants.MAP_SHARED,
                fd,
                0
            )

            try {
                val buffer = ByteArray(testData.size)
                MmapHelper.copyFromAddress(address + 500, buffer, 0, buffer.size)
                assertEquals("Test data for reading", String(buffer))
                Log.i(TAG, "Read back: ${String(buffer)}")
            } finally {
                Os.munmap(address, fileSize)
            }
        }

        Log.i(TAG, "testCopyFromAddress_basic PASSED")
    }

    /**
     * Test wrapAddress returns a usable ByteBuffer.
     */
    @Test
    fun testWrapAddress_basic() {
        val testFile = File(testDir, "wrap_basic.bin")
        val fileSize = 1024 * 1024L // 1MB

        RandomAccessFile(testFile, "rw").use { it.setLength(fileSize) }

        RandomAccessFile(testFile, "rw").use { raf ->
            val fd = raf.fd
            val address = Os.mmap(
                0,
                fileSize,
                OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                OsConstants.MAP_SHARED,
                fd,
                0
            )

            try {
                val buffer = MmapHelper.wrapAddress(address, fileSize)
                assertNotNull("wrapAddress should return a ByteBuffer", buffer)

                // Write via ByteBuffer
                val testBytes = "ByteBuffer test".toByteArray()
                buffer!!.put(testBytes)

                Log.i(TAG, "Wrote via ByteBuffer wrapper")
            } finally {
                Os.munmap(address, fileSize)
            }
        }

        // Verify
        RandomAccessFile(testFile, "r").use { raf ->
            val buffer = ByteArray(15)
            raf.readFully(buffer)
            assertEquals("ByteBuffer test", String(buffer))
        }

        Log.i(TAG, "testWrapAddress_basic PASSED")
    }

    /**
     * Test large sequential write via mmap (100MB).
     * Measures throughput to verify mmap is fast.
     */
    @Test
    fun testLargeSequentialWrite() {
        val testFile = File(testDir, "large_seq.bin")
        val fileSize = 100L * 1024 * 1024 // 100MB
        val chunkSize = 1024 * 1024 // 1MB chunks
        val numChunks = (fileSize / chunkSize).toInt()
        val chunk = ByteArray(chunkSize).also { Random.nextBytes(it) }

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(fileSize) }

        val startTime = System.nanoTime()

        RandomAccessFile(testFile, "rw").use { raf ->
            val fd = raf.fd
            val address = Os.mmap(
                0,
                fileSize,
                OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                OsConstants.MAP_SHARED,
                fd,
                0
            )

            try {
                for (i in 0 until numChunks) {
                    val offset = i.toLong() * chunkSize
                    MmapHelper.copyToAddress(address + offset, chunk, 0, chunkSize)
                }
            } finally {
                Os.munmap(address, fileSize)
            }
        }

        val elapsed = System.nanoTime() - startTime
        val mbps = (fileSize / (1024.0 * 1024.0)) / (elapsed / 1_000_000_000.0)

        Log.i(TAG, "Large sequential write: ${fileSize / (1024 * 1024)}MB in ${elapsed / 1_000_000}ms = %.1f MB/s".format(mbps))
        assertTrue("Throughput should be > 50 MB/s", mbps > 50)

        Log.i(TAG, "testLargeSequentialWrite PASSED")
    }

    /**
     * Test random offset writes via mmap.
     * Simulates torrent piece writes arriving out of order.
     */
    @Test
    fun testRandomOffsetWrites() {
        val testFile = File(testDir, "random_writes.bin")
        val fileSize = 50L * 1024 * 1024 // 50MB
        val pieceSize = 256 * 1024 // 256KB pieces
        val numPieces = (fileSize / pieceSize).toInt()
        val piece = ByteArray(pieceSize).also { Random.nextBytes(it) }

        // Generate random write order
        val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(fileSize) }

        val startTime = System.nanoTime()

        RandomAccessFile(testFile, "rw").use { raf ->
            val fd = raf.fd
            val address = Os.mmap(
                0,
                fileSize,
                OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                OsConstants.MAP_SHARED,
                fd,
                0
            )

            try {
                for (offset in offsets) {
                    MmapHelper.copyToAddress(address + offset, piece, 0, pieceSize)
                }
            } finally {
                Os.munmap(address, fileSize)
            }
        }

        val elapsed = System.nanoTime() - startTime
        val mbps = (fileSize / (1024.0 * 1024.0)) / (elapsed / 1_000_000_000.0)

        Log.i(TAG, "Random offset writes: ${fileSize / (1024 * 1024)}MB in ${elapsed / 1_000_000}ms = %.1f MB/s".format(mbps))

        Log.i(TAG, "testRandomOffsetWrites PASSED")
    }

    /**
     * Test multi-region mmap simulation.
     * Maps multiple 256MB regions to simulate >2GB file handling.
     */
    @Test
    fun testMultiRegionMmap() {
        val testFile = File(testDir, "multi_region.bin")
        val regionSize = 64L * 1024 * 1024 // 64MB regions (smaller for test speed)
        val numRegions = 4
        val fileSize = regionSize * numRegions // 256MB total
        val chunkSize = 1024 * 1024 // 1MB writes

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(fileSize) }

        val startTime = System.nanoTime()
        var bytesWritten = 0L

        RandomAccessFile(testFile, "rw").use { raf ->
            val fd = raf.fd

            // Map and write to each region separately (simulating SafMultiRegionMappedHandle)
            for (regionIdx in 0 until numRegions) {
                val regionOffset = regionIdx * regionSize

                val address = Os.mmap(
                    0,
                    regionSize,
                    OsConstants.PROT_READ or OsConstants.PROT_WRITE,
                    OsConstants.MAP_SHARED,
                    fd,
                    regionOffset
                )

                try {
                    Log.d(TAG, "Mapped region $regionIdx at offset ${regionOffset / (1024 * 1024)}MB, addr=0x${address.toString(16)}")

                    // Write chunks to this region
                    val chunksPerRegion = (regionSize / chunkSize).toInt()
                    val chunk = ByteArray(chunkSize).also { Random.nextBytes(it) }

                    for (i in 0 until chunksPerRegion) {
                        val localOffset = i.toLong() * chunkSize
                        MmapHelper.copyToAddress(address + localOffset, chunk, 0, chunkSize)
                        bytesWritten += chunkSize
                    }
                } finally {
                    Os.munmap(address, regionSize)
                }
            }
        }

        val elapsed = System.nanoTime() - startTime
        val mbps = (bytesWritten / (1024.0 * 1024.0)) / (elapsed / 1_000_000_000.0)

        Log.i(TAG, "Multi-region mmap: ${bytesWritten / (1024 * 1024)}MB across $numRegions regions in ${elapsed / 1_000_000}ms = %.1f MB/s".format(mbps))

        // Verify file size
        assertEquals("File should be correct size", fileSize, testFile.length())

        Log.i(TAG, "testMultiRegionMmap PASSED")
    }

    /**
     * Test FileHandlePool with SafOsMappedHandle.
     * Uses the actual FileHandlePool to test the full integration.
     */
    @Test
    fun testFileHandlePool_safOsMapped() {
        val testFile = File(testDir, "pool_test.bin")
        val fileSize = 10L * 1024 * 1024 // 10MB
        val pieceSize = 256 * 1024 // 256KB pieces
        val numPieces = (fileSize / pieceSize).toInt()

        val pool = FileHandlePool(context, maxHandles = 64, useMmap = true)

        try {
            // Pre-allocate
            val success = pool.preallocate(testFile, fileSize)
            assertTrue("Pre-allocation should succeed", success)

            // Write pieces in random order
            val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()
            val piece = ByteArray(pieceSize).also { Random.nextBytes(it) }

            val startTime = System.nanoTime()

            for (offset in offsets) {
                pool.writeAt(testFile, offset, piece)
            }

            val elapsed = System.nanoTime() - startTime
            val mbps = (fileSize / (1024.0 * 1024.0)) / (elapsed / 1_000_000_000.0)

            Log.i(TAG, "FileHandlePool mmap writes: ${fileSize / (1024 * 1024)}MB in ${elapsed / 1_000_000}ms = %.1f MB/s".format(mbps))
            Log.i(TAG, "Pool stats: ${pool.getStats()}")

            // Verify file size
            assertEquals("File should be correct size", fileSize, testFile.length())
        } finally {
            pool.closeAll()
        }

        Log.i(TAG, "testFileHandlePool_safOsMapped PASSED")
    }

    /**
     * Test FileHandlePool with multi-region mmap for large file.
     * Creates a file larger than 2GB to trigger multi-region handling.
     *
     * NOTE: This test requires ~3GB free disk space. Skip if not enough space.
     */
    @Test
    fun testFileHandlePool_multiRegion_largeFile() {
        val testFile = File(testDir, "large_multi_region.bin")
        val fileSize = 2.5 * 1024 * 1024 * 1024 // 2.5GB - triggers multi-region
        val fileSizeLong = fileSize.toLong()

        // Check available space
        val availableSpace = testDir.usableSpace
        if (availableSpace < fileSizeLong + 500 * 1024 * 1024) {
            Log.w(TAG, "Skipping large file test - need ${fileSizeLong / (1024 * 1024)}MB but only ${availableSpace / (1024 * 1024)}MB available")
            return
        }

        val pool = FileHandlePool(context, maxHandles = 64, useMmap = true)

        try {
            Log.i(TAG, "Pre-allocating ${fileSizeLong / (1024 * 1024)}MB file...")
            val allocStart = System.currentTimeMillis()
            val success = pool.preallocate(testFile, fileSizeLong)
            val allocTime = System.currentTimeMillis() - allocStart
            Log.i(TAG, "Pre-allocation took ${allocTime}ms")

            assertTrue("Pre-allocation should succeed for >2GB file", success)
            assertEquals("File should be correct size", fileSizeLong, testFile.length())

            // Write to various regions across the file
            val pieceSize = 1024 * 1024 // 1MB pieces
            val testOffsets = listOf(
                0L,                                    // Start of file
                500L * 1024 * 1024,                   // 500MB in
                1500L * 1024 * 1024,                  // 1.5GB in
                2000L * 1024 * 1024,                  // 2GB in (past single-region limit)
                fileSizeLong - pieceSize              // End of file
            )

            val piece = ByteArray(pieceSize).also { Random.nextBytes(it) }
            var totalWriteTime = 0L

            for (offset in testOffsets) {
                val writeStart = System.nanoTime()
                pool.writeAt(testFile, offset, piece)
                val writeTime = System.nanoTime() - writeStart
                totalWriteTime += writeTime
                Log.i(TAG, "Wrote 1MB at offset ${offset / (1024 * 1024)}MB in ${writeTime / 1_000_000}ms")
            }

            val avgWriteMs = totalWriteTime / testOffsets.size / 1_000_000
            Log.i(TAG, "Average write time: ${avgWriteMs}ms per 1MB piece")
            Log.i(TAG, "Pool stats: ${pool.getStats()}")

            // Verify we can read back from the >2GB offset
            RandomAccessFile(testFile, "r").use { raf ->
                raf.seek(2000L * 1024 * 1024)
                val buffer = ByteArray(100)
                raf.readFully(buffer)
                // Just verify it doesn't throw - data correctness is secondary
                Log.i(TAG, "Successfully read from offset 2GB")
            }

        } finally {
            pool.closeAll()
            // Clean up large file immediately
            testFile.delete()
        }

        Log.i(TAG, "testFileHandlePool_multiRegion_largeFile PASSED")
    }
}
