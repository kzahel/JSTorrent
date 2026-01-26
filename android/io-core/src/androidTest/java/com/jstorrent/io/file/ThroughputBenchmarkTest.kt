package com.jstorrent.io.file

import android.net.Uri
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.io.FileOutputStream
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.security.MessageDigest
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock
import kotlin.random.Random

/**
 * Throughput benchmarks to identify disk I/O bottlenecks.
 *
 * Tests various write strategies to find maximum achievable throughput:
 * - Sequential vs random offset writes
 * - Open/close per write vs keep handle open
 * - Batching writes before flush
 * - Single file vs multiple files
 * - Effect of fsync
 *
 * Run with: ./gradlew :io-core:connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=com.jstorrent.io.file.ThroughputBenchmarkTest
 */
@RunWith(AndroidJUnit4::class)
class ThroughputBenchmarkTest {

    companion object {
        private const val TAG = "ThroughputBench"

        // Piece sizes to test
        private const val PIECE_256KB = 256 * 1024
        private const val PIECE_512KB = 512 * 1024
        private const val PIECE_1MB = 1024 * 1024

        // Test parameters
        private const val TOTAL_DATA_MB = 100  // Write 100MB total per test
        private const val WARMUP_WRITES = 10   // Warmup writes to prime caches
    }

    private lateinit var testDir: File
    private lateinit var rootUri: Uri
    private lateinit var context: android.content.Context

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        testDir = File(context.filesDir, "throughput_bench_${System.currentTimeMillis()}")
        testDir.mkdirs()
        rootUri = Uri.parse("file://${testDir.absolutePath}")
        Log.i(TAG, "=".repeat(60))
        Log.i(TAG, "Test directory: ${testDir.absolutePath}")
    }

    @After
    fun tearDown() {
        testDir.deleteRecursively()
    }

    // =========================================================================
    // BASELINE: Raw file I/O without any abstraction
    // =========================================================================

    /**
     * Baseline: Sequential writes with single open handle.
     * This is the theoretical maximum throughput.
     */
    @Test
    fun baseline_sequentialWrite_singleHandle() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "baseline_seq.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "BASELINE: Sequential write, single handle, ${pieceSize/1024}KB pieces")

        // Warmup
        RandomAccessFile(testFile, "rw").use { raf ->
            repeat(WARMUP_WRITES) { i ->
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
            }
        }
        testFile.delete()

        // Actual test
        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("baseline_sequential_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * Baseline: Sequential writes with open/close per write.
     * Shows the cost of file handle overhead.
     */
    @Test
    fun baseline_sequentialWrite_openCloseEach() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "baseline_oc.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "BASELINE: Sequential write, open/close each, ${pieceSize/1024}KB pieces")

        // Warmup
        repeat(WARMUP_WRITES) { i ->
            RandomAccessFile(testFile, "rw").use { raf ->
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
            }
        }
        testFile.delete()

        // Actual test
        val start = System.nanoTime()
        for (i in 0 until numPieces) {
            RandomAccessFile(testFile, "rw").use { raf ->
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("baseline_openclose_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * Baseline: Random offset writes with single handle.
     * Shows the cost of random seeks.
     */
    @Test
    fun baseline_randomWrite_singleHandle() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "baseline_rand.bin")
        val data = randomData(pieceSize)

        // Pre-allocate file
        RandomAccessFile(testFile, "rw").use { raf ->
            raf.setLength(numPieces.toLong() * pieceSize)
        }

        // Generate random write order
        val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "BASELINE: Random write, single handle, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (offset in offsets) {
                raf.seek(offset)
                raf.write(data)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("baseline_random_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // FSYNC TESTS: Effect of forcing data to storage
    // =========================================================================

    /**
     * Sequential writes with fsync after each write.
     * Shows the true cost of guaranteed durability.
     */
    @Test
    fun fsync_afterEachWrite() {
        val pieceSize = PIECE_1MB
        val numPieces = 50  // Fewer pieces since fsync is slow
        val testFile = File(testDir, "fsync_each.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "FSYNC: After each write, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
                raf.fd.sync()  // Force to storage
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("fsync_each_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * Sequential writes with fsync every N writes.
     * Shows benefit of batched fsync.
     */
    @Test
    fun fsync_everyNWrites() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "fsync_batch.bin")
        val data = randomData(pieceSize)
        val syncEvery = 10  // Sync every 10 pieces

        Log.i(TAG, "FSYNC: Every $syncEvery writes, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
                if ((i + 1) % syncEvery == 0) {
                    raf.fd.sync()
                }
            }
            raf.fd.sync()  // Final sync
        }
        val elapsed = System.nanoTime() - start

        logResult("fsync_every${syncEvery}_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // BATCHING TESTS: Accumulate writes then flush
    // =========================================================================

    /**
     * Batch random writes, sort by offset, write sequentially.
     * Simulates reordering a write queue before flushing.
     */
    @Test
    fun batched_sortedWrites() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "batched_sorted.bin")
        val data = randomData(pieceSize)

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(numPieces.toLong() * pieceSize) }

        // Simulate pieces arriving in random order
        val arrivals = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        // Batch size: accumulate this many before sorting and writing
        val batchSize = 10

        Log.i(TAG, "BATCHED: Sort $batchSize writes before flush, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            val batch = mutableListOf<Long>()
            for (offset in arrivals) {
                batch.add(offset)
                if (batch.size >= batchSize) {
                    // Sort and write
                    batch.sort()
                    for (off in batch) {
                        raf.seek(off)
                        raf.write(data)
                    }
                    batch.clear()
                }
            }
            // Flush remaining
            if (batch.isNotEmpty()) {
                batch.sort()
                for (off in batch) {
                    raf.seek(off)
                    raf.write(data)
                }
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("batched_sorted_${batchSize}_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * Batch random writes without sorting (just accumulate).
     * Control test to show sorting benefit.
     */
    @Test
    fun batched_unsortedWrites() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "batched_unsorted.bin")
        val data = randomData(pieceSize)

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(numPieces.toLong() * pieceSize) }

        val arrivals = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()
        val batchSize = 10

        Log.i(TAG, "BATCHED: Unsorted $batchSize writes before flush, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            val batch = mutableListOf<Long>()
            for (offset in arrivals) {
                batch.add(offset)
                if (batch.size >= batchSize) {
                    // Write in arrival order (no sort)
                    for (off in batch) {
                        raf.seek(off)
                        raf.write(data)
                    }
                    batch.clear()
                }
            }
            if (batch.isNotEmpty()) {
                for (off in batch) {
                    raf.seek(off)
                    raf.write(data)
                }
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("batched_unsorted_${batchSize}_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // MULTI-FILE TESTS: Writing to multiple files concurrently
    // =========================================================================

    /**
     * Multi-file writes with handles kept open.
     * Simulates multi-file torrent download.
     */
    @Test
    fun multiFile_pooledHandles() {
        val pieceSize = PIECE_1MB
        val numFiles = 5
        val piecesPerFile = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize / numFiles
        val data = randomData(pieceSize)

        // Create files
        val files = (0 until numFiles).map { File(testDir, "multi_$it.bin") }
        val handles = files.map { RandomAccessFile(it, "rw") }

        // Generate random writes across all files
        data class WriteOp(val fileIdx: Int, val offset: Long)
        val writes = mutableListOf<WriteOp>()
        for (f in 0 until numFiles) {
            for (p in 0 until piecesPerFile) {
                writes.add(WriteOp(f, p.toLong() * pieceSize))
            }
        }
        writes.shuffle()

        Log.i(TAG, "MULTI-FILE: $numFiles files, pooled handles, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (write in writes) {
            handles[write.fileIdx].seek(write.offset)
            handles[write.fileIdx].write(data)
        }
        val elapsed = System.nanoTime() - start

        handles.forEach { it.close() }

        logResult("multifile_pooled_${numFiles}f_1MB", writes.size.toLong() * pieceSize, elapsed)
    }

    /**
     * Multi-file writes sorted by file then offset.
     * Shows benefit of grouping writes by file.
     */
    @Test
    fun multiFile_sortedByFile() {
        val pieceSize = PIECE_1MB
        val numFiles = 5
        val piecesPerFile = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize / numFiles
        val data = randomData(pieceSize)

        val files = (0 until numFiles).map { File(testDir, "multisort_$it.bin") }
        val handles = files.map { RandomAccessFile(it, "rw") }

        data class WriteOp(val fileIdx: Int, val offset: Long)
        val writes = mutableListOf<WriteOp>()
        for (f in 0 until numFiles) {
            for (p in 0 until piecesPerFile) {
                writes.add(WriteOp(f, p.toLong() * pieceSize))
            }
        }
        writes.shuffle()

        // Sort by file, then by offset within file
        val sorted = writes.sortedWith(compareBy({ it.fileIdx }, { it.offset }))

        Log.i(TAG, "MULTI-FILE: $numFiles files, sorted by file+offset, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (write in sorted) {
            handles[write.fileIdx].seek(write.offset)
            handles[write.fileIdx].write(data)
        }
        val elapsed = System.nanoTime() - start

        handles.forEach { it.close() }

        logResult("multifile_sorted_${numFiles}f_1MB", writes.size.toLong() * pieceSize, elapsed)
    }

    // =========================================================================
    // SAF TESTS: Storage Access Framework overhead
    // =========================================================================

    /**
     * Sequential streaming copy from app storage to SAF.
     * This simulates: write fast to app storage, then bulk copy to SAF.
     *
     * To test with real SAF, run manually after granting SAF permission:
     * adb shell am instrument -w -e class com.jstorrent.io.file.ThroughputBenchmarkTest#saf_sequentialStreamCopy com.jstorrent.io.core.test/androidx.test.runner.AndroidJUnitRunner
     */
    @Test
    fun saf_sequentialStreamCopy() {
        val totalMB = 100
        val bufferSize = 1024 * 1024  // 1MB buffer for streaming
        val sourceFile = File(testDir, "source_for_copy.bin")
        val destFile = File(testDir, "dest_copy.bin")

        // Create source file with random data
        Log.i(TAG, "SAF SEQUENTIAL: Creating ${totalMB}MB source file...")
        val data = randomData(bufferSize)
        RandomAccessFile(sourceFile, "rw").use { raf ->
            repeat(totalMB) {
                raf.write(data)
            }
        }

        Log.i(TAG, "SAF SEQUENTIAL: Streaming copy ${totalMB}MB with ${bufferSize/1024}KB buffer")

        // Time the sequential streaming copy
        val start = System.nanoTime()
        sourceFile.inputStream().buffered(bufferSize).use { input ->
            destFile.outputStream().buffered(bufferSize).use { output ->
                val buffer = ByteArray(bufferSize)
                var totalCopied = 0L
                while (true) {
                    val read = input.read(buffer)
                    if (read == -1) break
                    output.write(buffer, 0, read)
                    totalCopied += read
                }
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("saf_sequential_stream_copy", totalMB.toLong() * 1024 * 1024, elapsed)
    }

    /**
     * Random writes to same file size - compare to sequential copy.
     */
    @Test
    fun saf_randomWritesSameSize() {
        val totalMB = 100
        val pieceSize = 256 * 1024  // 256KB pieces (typical torrent)
        val numPieces = (totalMB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "random_writes.bin")
        val data = randomData(pieceSize)

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(totalMB.toLong() * 1024 * 1024) }

        // Random offsets
        val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "SAF RANDOM: ${totalMB}MB with ${pieceSize/1024}KB pieces, $numPieces writes")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (offset in offsets) {
                raf.seek(offset)
                raf.write(data)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("saf_random_writes_256kb", totalMB.toLong() * 1024 * 1024, elapsed)
    }

    /**
     * SAF writes via FileManagerImpl without handle pool.
     */
    @Test
    fun saf_noPool() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val data = randomData(pieceSize)
        val fileManager = FileManagerImpl(context, fileHandlePool = null)

        Log.i(TAG, "SAF: FileManagerImpl without pool, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (i in 0 until numPieces) {
            fileManager.write(rootUri, "saf_nopool.bin", i.toLong() * pieceSize, data)
        }
        val elapsed = System.nanoTime() - start

        logResult("saf_nopool_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * SAF writes via FileManagerImpl with handle pool.
     */
    @Test
    fun saf_withPool() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val data = randomData(pieceSize)
        val pool = FileHandlePool(context, maxHandles = 64)
        val fileManager = FileManagerImpl(context, fileHandlePool = pool)

        Log.i(TAG, "SAF: FileManagerImpl with pool, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (i in 0 until numPieces) {
            fileManager.write(rootUri, "saf_pool.bin", i.toLong() * pieceSize, data)
        }
        val elapsed = System.nanoTime() - start

        pool.closeAll()
        logResult("saf_withpool_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // FILECHANNEL TESTS: NIO vs RandomAccessFile
    // =========================================================================

    /**
     * FileChannel writes (NIO).
     * May have better buffering behavior.
     */
    @Test
    fun nio_fileChannel_sequential() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "nio_channel.bin")
        val data = randomData(pieceSize)
        val buffer = ByteBuffer.wrap(data)

        Log.i(TAG, "NIO: FileChannel sequential, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        FileOutputStream(testFile).channel.use { channel ->
            for (i in 0 until numPieces) {
                buffer.rewind()
                channel.position(i.toLong() * pieceSize)
                channel.write(buffer)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("nio_channel_seq_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * FileChannel with force(true) after each write.
     */
    @Test
    fun nio_fileChannel_withForce() {
        val pieceSize = PIECE_1MB
        val numPieces = 50  // Fewer since force is slow
        val testFile = File(testDir, "nio_force.bin")
        val data = randomData(pieceSize)
        val buffer = ByteBuffer.wrap(data)

        Log.i(TAG, "NIO: FileChannel with force(true), ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        FileOutputStream(testFile).channel.use { channel ->
            for (i in 0 until numPieces) {
                buffer.rewind()
                channel.position(i.toLong() * pieceSize)
                channel.write(buffer)
                channel.force(true)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("nio_force_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // PARALLEL WRITE TESTS: Multiple threads writing
    // =========================================================================

    /**
     * Parallel writes to different files.
     */
    @Test
    fun parallel_multipleFiles() {
        val pieceSize = PIECE_1MB
        val numThreads = 4
        val piecesPerThread = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize / numThreads
        val data = randomData(pieceSize)
        val executor = Executors.newFixedThreadPool(numThreads)
        val latch = CountDownLatch(numThreads)
        val totalBytes = AtomicLong(0)

        Log.i(TAG, "PARALLEL: $numThreads threads, separate files, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (t in 0 until numThreads) {
            executor.submit {
                try {
                    val file = File(testDir, "parallel_$t.bin")
                    RandomAccessFile(file, "rw").use { raf ->
                        for (i in 0 until piecesPerThread) {
                            raf.seek(i.toLong() * pieceSize)
                            raf.write(data)
                            totalBytes.addAndGet(pieceSize.toLong())
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("parallel_${numThreads}t_1MB", totalBytes.get(), elapsed)
    }

    // =========================================================================
    // PIECE SIZE COMPARISON
    // =========================================================================

    @Test
    fun pieceSize_256KB() {
        benchmarkPieceSize(PIECE_256KB)
    }

    @Test
    fun pieceSize_512KB() {
        benchmarkPieceSize(PIECE_512KB)
    }

    @Test
    fun pieceSize_1MB() {
        benchmarkPieceSize(PIECE_1MB)
    }

    private fun benchmarkPieceSize(pieceSize: Int) {
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "piecesize_${pieceSize/1024}kb.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "PIECE SIZE: ${pieceSize/1024}KB, $numPieces pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("piecesize_${pieceSize/1024}kb", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // HASH TESTS: SHA-1 hashing overhead
    // =========================================================================

    /**
     * SHA-1 hash only (no disk I/O).
     * Shows pure hashing throughput.
     */
    @Test
    fun hash_sha1Only() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val data = randomData(pieceSize)

        Log.i(TAG, "HASH: SHA-1 only, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (i in 0 until numPieces) {
            MessageDigest.getInstance("SHA-1").digest(data)
        }
        val elapsed = System.nanoTime() - start

        logResult("hash_sha1_only_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * Hash then write sequentially (single thread).
     * Simulates verified write without parallelism.
     */
    @Test
    fun hashAndWrite_sequential() {
        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "hash_write_seq.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "HASH+WRITE: Sequential, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                // Hash first (like verified write)
                MessageDigest.getInstance("SHA-1").digest(data)
                // Then write
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("hash_write_seq_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    /**
     * Hash then write with fsync (true durability).
     */
    @Test
    fun hashAndWrite_withFsync() {
        val pieceSize = PIECE_1MB
        val numPieces = 50  // Fewer since fsync is slow
        val testFile = File(testDir, "hash_write_fsync.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "HASH+WRITE+FSYNC: ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                MessageDigest.getInstance("SHA-1").digest(data)
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
                raf.fd.sync()
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("hash_write_fsync_1MB", numPieces * pieceSize.toLong(), elapsed)
    }

    // =========================================================================
    // 4-WORKER PATTERN: Simulating actual torrent write pattern
    // =========================================================================

    /**
     * 4 workers writing to SAME file at random offsets.
     * This is the torrent pattern - multiple pieces to same file concurrently.
     * Uses a shared lock on the file handle.
     */
    @Test
    fun workers4_sameFile_withLock() {
        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val piecesPerWorker = totalPieces / numWorkers
        val testFile = File(testDir, "workers4_same.bin")
        val data = randomData(pieceSize)

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(totalPieces.toLong() * pieceSize) }

        // Shared handle with lock
        val raf = RandomAccessFile(testFile, "rw")
        val lock = ReentrantLock()

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        // Distribute pieces randomly among workers
        val allOffsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "4-WORKER: Same file, shared handle with lock, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (w in 0 until numWorkers) {
            val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
            executor.submit {
                try {
                    for (offset in workerOffsets) {
                        lock.withLock {
                            raf.seek(offset)
                            raf.write(data)
                        }
                        totalBytes.addAndGet(pieceSize.toLong())
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        raf.close()
        executor.shutdown()
        logResult("workers4_samefile_lock_1MB", totalBytes.get(), elapsed)
    }

    /**
     * 4 workers writing to SAME file, each with own handle (no lock needed).
     * RandomAccessFile is thread-safe for independent operations.
     */
    @Test
    fun workers4_sameFile_separateHandles() {
        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val piecesPerWorker = totalPieces / numWorkers
        val testFile = File(testDir, "workers4_same_sep.bin")
        val data = randomData(pieceSize)

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(totalPieces.toLong() * pieceSize) }

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        val allOffsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "4-WORKER: Same file, separate handles (no lock), ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (w in 0 until numWorkers) {
            val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
            executor.submit {
                try {
                    // Each worker gets own handle
                    RandomAccessFile(testFile, "rw").use { raf ->
                        for (offset in workerOffsets) {
                            raf.seek(offset)
                            raf.write(data)
                            totalBytes.addAndGet(pieceSize.toLong())
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("workers4_samefile_sephandles_1MB", totalBytes.get(), elapsed)
    }

    /**
     * 4 workers with hash + write to same file.
     * Most realistic torrent simulation.
     */
    @Test
    fun workers4_hashAndWrite_sameFile() {
        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val piecesPerWorker = totalPieces / numWorkers
        val testFile = File(testDir, "workers4_hash.bin")
        val data = randomData(pieceSize)

        RandomAccessFile(testFile, "rw").use { it.setLength(totalPieces.toLong() * pieceSize) }

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        val allOffsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "4-WORKER: Hash+write, same file, separate handles, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (w in 0 until numWorkers) {
            val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
            executor.submit {
                try {
                    RandomAccessFile(testFile, "rw").use { raf ->
                        for (offset in workerOffsets) {
                            // Hash first
                            MessageDigest.getInstance("SHA-1").digest(data)
                            // Then write
                            raf.seek(offset)
                            raf.write(data)
                            totalBytes.addAndGet(pieceSize.toLong())
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("workers4_hash_write_1MB", totalBytes.get(), elapsed)
    }

    /**
     * 4 workers with hash + write + fsync to same file.
     * True durability with parallelism.
     */
    @Test
    fun workers4_hashWriteFsync_sameFile() {
        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = 50  // Fewer since fsync is slow
        val piecesPerWorker = totalPieces / numWorkers
        val testFile = File(testDir, "workers4_fsync.bin")
        val data = randomData(pieceSize)

        RandomAccessFile(testFile, "rw").use { it.setLength(totalPieces.toLong() * pieceSize) }

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        val allOffsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "4-WORKER: Hash+write+fsync, same file, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (w in 0 until numWorkers) {
            val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
            executor.submit {
                try {
                    RandomAccessFile(testFile, "rw").use { raf ->
                        for (offset in workerOffsets) {
                            MessageDigest.getInstance("SHA-1").digest(data)
                            raf.seek(offset)
                            raf.write(data)
                            raf.fd.sync()
                            totalBytes.addAndGet(pieceSize.toLong())
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("workers4_hash_write_fsync_1MB", totalBytes.get(), elapsed)
    }

    // =========================================================================
    // QUEUE-BASED PATTERN: Producer-consumer like torrent engine
    // =========================================================================

    /**
     * Producer queues writes, 4 consumer workers process them.
     * Simulates the async write pattern with a queue.
     */
    @Test
    fun queueBased_4workers() {
        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "queue_based.bin")
        val data = randomData(pieceSize)

        RandomAccessFile(testFile, "rw").use { it.setLength(totalPieces.toLong() * pieceSize) }

        data class WriteJob(val offset: Long, val data: ByteArray, val isPoison: Boolean = false)
        val queue = LinkedBlockingQueue<WriteJob>(100)  // Bounded queue for backpressure
        val poisonPill = WriteJob(0, ByteArray(0), isPoison = true)

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        Log.i(TAG, "QUEUE: 4 workers, bounded queue(100), hash+write, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()

        // Start workers
        for (w in 0 until numWorkers) {
            executor.submit {
                try {
                    RandomAccessFile(testFile, "rw").use { raf ->
                        while (true) {
                            val job = queue.take()
                            if (job.isPoison) break
                            // Hash
                            MessageDigest.getInstance("SHA-1").digest(job.data)
                            // Write
                            raf.seek(job.offset)
                            raf.write(job.data)
                            totalBytes.addAndGet(job.data.size.toLong())
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }

        // Producer: queue all writes (will block if queue full = backpressure)
        val offsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()
        for (offset in offsets) {
            queue.put(WriteJob(offset, data))
        }

        // Send poison pills
        repeat(numWorkers) { queue.put(poisonPill) }

        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("queue_4workers_1MB", totalBytes.get(), elapsed)
    }

    /**
     * Queue-based with fsync every N writes per worker.
     */
    @Test
    fun queueBased_4workers_batchedFsync() {
        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "queue_fsync.bin")
        val data = randomData(pieceSize)
        val fsyncEvery = 10

        RandomAccessFile(testFile, "rw").use { it.setLength(totalPieces.toLong() * pieceSize) }

        data class WriteJob(val offset: Long, val data: ByteArray, val isPoison: Boolean = false)
        val queue = LinkedBlockingQueue<WriteJob>(100)
        val poisonPill = WriteJob(0, ByteArray(0), isPoison = true)

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        Log.i(TAG, "QUEUE: 4 workers, fsync every $fsyncEvery, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()

        for (w in 0 until numWorkers) {
            executor.submit {
                try {
                    RandomAccessFile(testFile, "rw").use { raf ->
                        var writeCount = 0
                        while (true) {
                            val job = queue.take()
                            if (job.isPoison) break
                            MessageDigest.getInstance("SHA-1").digest(job.data)
                            raf.seek(job.offset)
                            raf.write(job.data)
                            totalBytes.addAndGet(job.data.size.toLong())

                            writeCount++
                            if (writeCount % fsyncEvery == 0) {
                                raf.fd.sync()
                            }
                        }
                        raf.fd.sync()  // Final sync
                    }
                } finally {
                    latch.countDown()
                }
            }
        }

        val offsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()
        for (offset in offsets) {
            queue.put(WriteJob(offset, data))
        }
        repeat(numWorkers) { queue.put(poisonPill) }

        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("queue_4workers_fsync${fsyncEvery}_1MB", totalBytes.get(), elapsed)
    }

    // =========================================================================
    // LARGE WRITE STRESS TESTS: Overflow page cache, hit real disk
    // =========================================================================

    /**
     * Write 500MB sequentially - large enough to overflow page cache.
     * This should show real sustained disk throughput.
     */
    @Test
    fun stress_500MB_sequential() {
        val pieceSize = PIECE_1MB
        val totalMB = 500
        val numPieces = (totalMB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "stress_500mb.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "STRESS: 500MB sequential write, ${pieceSize/1024}KB pieces, $numPieces writes")

        val start = System.nanoTime()
        var lastLogTime = start
        var bytesWritten = 0L

        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
                bytesWritten += pieceSize

                // Log progress every 100MB
                val now = System.nanoTime()
                if (bytesWritten % (100 * 1024 * 1024) == 0L) {
                    val elapsedSec = (now - start) / 1_000_000_000.0
                    val mbps = (bytesWritten / (1024.0 * 1024.0)) / elapsedSec
                    Log.i(TAG, "  Progress: ${bytesWritten / (1024*1024)}MB written, %.1f MB/s avg".format(mbps))
                }
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("stress_500MB_seq", bytesWritten, elapsed)
    }

    /**
     * Write 500MB with fsync every 50MB.
     * Forces periodic flushes to show sustained disk speed.
     */
    @Test
    fun stress_500MB_batchedFsync() {
        val pieceSize = PIECE_1MB
        val totalMB = 500
        val numPieces = (totalMB * 1024 * 1024) / pieceSize
        val testFile = File(testDir, "stress_500mb_fsync.bin")
        val data = randomData(pieceSize)
        val syncEveryMB = 50
        val syncEveryPieces = (syncEveryMB * 1024 * 1024) / pieceSize

        Log.i(TAG, "STRESS: 500MB with fsync every ${syncEveryMB}MB, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        var bytesWritten = 0L

        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i.toLong() * pieceSize)
                raf.write(data)
                bytesWritten += pieceSize

                if ((i + 1) % syncEveryPieces == 0) {
                    val syncStart = System.nanoTime()
                    raf.fd.sync()
                    val syncTime = (System.nanoTime() - syncStart) / 1_000_000
                    val elapsedSec = (System.nanoTime() - start) / 1_000_000_000.0
                    val mbps = (bytesWritten / (1024.0 * 1024.0)) / elapsedSec
                    Log.i(TAG, "  Synced at ${bytesWritten / (1024*1024)}MB, sync took ${syncTime}ms, %.1f MB/s avg".format(mbps))
                }
            }
            raf.fd.sync()  // Final sync
        }
        val elapsed = System.nanoTime() - start

        logResult("stress_500MB_fsync50", bytesWritten, elapsed)
    }

    /**
     * Write 500MB random offsets - simulates torrent piece arrival pattern.
     * 4 workers, each writing to different random offsets.
     */
    @Test
    fun stress_500MB_4workers_random() {
        val pieceSize = PIECE_1MB
        val totalMB = 500
        val numPieces = (totalMB * 1024 * 1024) / pieceSize
        val numWorkers = 4
        val piecesPerWorker = numPieces / numWorkers
        val testFile = File(testDir, "stress_500mb_4w.bin")
        val data = randomData(pieceSize)

        // Pre-allocate
        RandomAccessFile(testFile, "rw").use { it.setLength(numPieces.toLong() * pieceSize) }

        // Shuffle offsets and distribute to workers
        val allOffsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        Log.i(TAG, "STRESS: 500MB, 4 workers, random offsets, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (w in 0 until numWorkers) {
            val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
            executor.submit {
                try {
                    RandomAccessFile(testFile, "rw").use { raf ->
                        for (offset in workerOffsets) {
                            raf.seek(offset)
                            raf.write(data)
                            totalBytes.addAndGet(pieceSize.toLong())
                        }
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("stress_500MB_4w_random", totalBytes.get(), elapsed)
    }

    /**
     * Write 500MB with 4 workers + hash + fsync every 50 pieces per worker.
     * This is the closest to actual torrent behavior.
     */
    @Test
    fun stress_500MB_4workers_hashWriteFsync() {
        val pieceSize = PIECE_1MB
        val totalMB = 500
        val numPieces = (totalMB * 1024 * 1024) / pieceSize
        val numWorkers = 4
        val piecesPerWorker = numPieces / numWorkers
        val testFile = File(testDir, "stress_500mb_4w_fsync.bin")
        val data = randomData(pieceSize)
        val fsyncEvery = 50  // Fsync every 50 pieces per worker

        RandomAccessFile(testFile, "rw").use { it.setLength(numPieces.toLong() * pieceSize) }

        val allOffsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        val executor = Executors.newFixedThreadPool(numWorkers)
        val latch = CountDownLatch(numWorkers)
        val totalBytes = AtomicLong(0)

        Log.i(TAG, "STRESS: 500MB, 4 workers, hash+write+fsync every $fsyncEvery, ${pieceSize/1024}KB pieces")

        val start = System.nanoTime()
        for (w in 0 until numWorkers) {
            val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
            executor.submit {
                try {
                    RandomAccessFile(testFile, "rw").use { raf ->
                        var writeCount = 0
                        for (offset in workerOffsets) {
                            // Hash
                            MessageDigest.getInstance("SHA-1").digest(data)
                            // Write
                            raf.seek(offset)
                            raf.write(data)
                            totalBytes.addAndGet(pieceSize.toLong())

                            writeCount++
                            if (writeCount % fsyncEvery == 0) {
                                raf.fd.sync()
                            }
                        }
                        raf.fd.sync()  // Final sync
                    }
                } finally {
                    latch.countDown()
                }
            }
        }
        latch.await()
        val elapsed = System.nanoTime() - start

        executor.shutdown()
        logResult("stress_500MB_4w_hash_fsync50", totalBytes.get(), elapsed)
    }

    /**
     * Write 1GB to truly stress the disk and ensure we're past any caching.
     * Single thread, sequential, no fsync - see when throughput drops.
     */
    @Test
    fun stress_1GB_sequential() {
        val pieceSize = PIECE_1MB
        val totalMB = 1024  // 1GB
        val numPieces = (totalMB * 1024 * 1024L) / pieceSize
        val testFile = File(testDir, "stress_1gb.bin")
        val data = randomData(pieceSize)

        Log.i(TAG, "STRESS: 1GB sequential write, ${pieceSize/1024}KB pieces, $numPieces writes")

        val start = System.nanoTime()
        var bytesWritten = 0L

        RandomAccessFile(testFile, "rw").use { raf ->
            for (i in 0 until numPieces) {
                raf.seek(i * pieceSize)
                raf.write(data)
                bytesWritten += pieceSize

                // Log progress every 200MB
                if (bytesWritten % (200 * 1024 * 1024) == 0L) {
                    val now = System.nanoTime()
                    val elapsedSec = (now - start) / 1_000_000_000.0
                    val mbps = (bytesWritten / (1024.0 * 1024.0)) / elapsedSec
                    Log.i(TAG, "  Progress: ${bytesWritten / (1024*1024)}MB written, %.1f MB/s avg".format(mbps))
                }
            }
        }
        val elapsed = System.nanoTime() - start

        logResult("stress_1GB_seq", bytesWritten, elapsed)
    }

    // =========================================================================
    // REAL SAF TESTS: ContentResolver via MediaStore (actual content:// URIs)
    // =========================================================================

    /**
     * Write to Downloads folder via MediaStore (content:// URI).
     * This is the REAL SAF code path - ContentResolver.openFileDescriptor().
     *
     * Compares to file:// writes to isolate SAF overhead.
     */
    @Test
    fun realSaf_mediaStore_sequentialWrite() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val numPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val data = randomData(pieceSize)

        // Create a file in Downloads via MediaStore (content:// URI)
        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "throughput_test_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri == null) {
            Log.e(TAG, "REAL SAF: Failed to create MediaStore entry")
            return
        }

        Log.i(TAG, "REAL SAF SEQUENTIAL: MediaStore content:// URI, ${pieceSize/1024}KB pieces")
        Log.i(TAG, "  URI: $contentUri")

        try {
            val start = System.nanoTime()
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).use { fos ->
                    val channel = fos.channel
                    for (i in 0 until numPieces) {
                        channel.position(i.toLong() * pieceSize)
                        channel.write(ByteBuffer.wrap(data))
                    }
                }
            }
            val elapsed = System.nanoTime() - start

            logResult("realSaf_mediaStore_seq_1MB", numPieces * pieceSize.toLong(), elapsed)
        } finally {
            // Clean up
            try {
                resolver.delete(contentUri, null, null)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete test file", e)
            }
        }
    }

    /**
     * Write to Downloads via MediaStore with mmap via Os.mmap().
     * Tests if mmap helps for content:// URIs.
     */
    @Test
    fun realSaf_mediaStore_mmap() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val totalMB = TOTAL_DATA_MB
        val totalSize = totalMB.toLong() * 1024 * 1024
        val numPieces = totalSize.toInt() / pieceSize
        val data = randomData(pieceSize)

        // Create file in Downloads
        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "throughput_mmap_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri == null) {
            Log.e(TAG, "REAL SAF MMAP: Failed to create MediaStore entry")
            return
        }

        Log.i(TAG, "REAL SAF MMAP: MediaStore with Os.mmap(), ${pieceSize/1024}KB pieces, ${totalMB}MB total")

        try {
            // Pre-allocate the file
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                    channel.truncate(totalSize)
                    // Write byte at end to force allocation
                    channel.position(totalSize - 1)
                    channel.write(ByteBuffer.wrap(byteArrayOf(0)))
                }
            }

            val start = System.nanoTime()
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                // Use Os.mmap directly (like FileHandlePool does for SAF)
                val address = android.system.Os.mmap(
                    0,
                    totalSize,
                    android.system.OsConstants.PROT_READ or android.system.OsConstants.PROT_WRITE,
                    android.system.OsConstants.MAP_SHARED,
                    pfd.fileDescriptor,
                    0
                )

                try {
                    // Random write order to simulate torrent
                    val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

                    for (offset in offsets) {
                        MmapHelper.copyToAddress(address + offset, data, 0, data.size)
                    }
                } finally {
                    android.system.Os.munmap(address, totalSize)
                }
            }
            val elapsed = System.nanoTime() - start

            logResult("realSaf_mediaStore_mmap_1MB", totalSize, elapsed)
        } finally {
            try {
                resolver.delete(contentUri, null, null)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete test file", e)
            }
        }
    }

    /**
     * Write to Downloads via MediaStore - random offsets, single FD kept open.
     * This simulates torrent piece arrival pattern over SAF.
     */
    @Test
    fun realSaf_mediaStore_randomWrites() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val totalMB = TOTAL_DATA_MB
        val totalSize = totalMB.toLong() * 1024 * 1024
        val numPieces = totalSize.toInt() / pieceSize
        val data = randomData(pieceSize)

        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "throughput_random_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri == null) {
            Log.e(TAG, "REAL SAF RANDOM: Failed to create MediaStore entry")
            return
        }

        Log.i(TAG, "REAL SAF RANDOM: MediaStore random writes, ${pieceSize/1024}KB pieces, ${totalMB}MB total")

        try {
            // Pre-allocate
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                    channel.truncate(totalSize)
                    channel.position(totalSize - 1)
                    channel.write(ByteBuffer.wrap(byteArrayOf(0)))
                }
            }

            // Random write order
            val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

            val start = System.nanoTime()
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                    for (offset in offsets) {
                        channel.position(offset)
                        channel.write(ByteBuffer.wrap(data))
                    }
                }
            }
            val elapsed = System.nanoTime() - start

            logResult("realSaf_mediaStore_random_1MB", totalSize, elapsed)
        } finally {
            try {
                resolver.delete(contentUri, null, null)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete test file", e)
            }
        }
    }

    /**
     * Test the "Flud strategy": Write to app-private temp file first,
     * then copy to SAF destination. This tests if staging helps.
     */
    @Test
    fun realSaf_tempFileThenCopy() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val totalMB = TOTAL_DATA_MB
        val totalSize = totalMB.toLong() * 1024 * 1024
        val numPieces = totalSize.toInt() / pieceSize
        val data = randomData(pieceSize)

        // Create temp file in app-private storage
        val tempFile = File(testDir, "temp_staging_${System.currentTimeMillis()}.bin")

        // Create destination in Downloads via MediaStore
        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "throughput_flud_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri == null) {
            Log.e(TAG, "FLUD STRATEGY: Failed to create MediaStore entry")
            return
        }

        Log.i(TAG, "FLUD STRATEGY: Write to temp file:// first, then copy to content://, ${pieceSize/1024}KB pieces, ${totalMB}MB total")

        try {
            // Random write order (simulating torrent pieces)
            val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

            // Phase 1: Write to temp file (should be fast)
            val writeStart = System.nanoTime()
            RandomAccessFile(tempFile, "rw").use { raf ->
                raf.setLength(totalSize)  // Pre-allocate
                for (offset in offsets) {
                    raf.seek(offset)
                    raf.write(data)
                }
            }
            val writeElapsed = System.nanoTime() - writeStart

            val writeMbps = (totalSize / (1024.0 * 1024.0)) / (writeElapsed / 1_000_000_000.0)
            Log.i(TAG, "  Phase 1 (temp write): %.1f MB/s".format(writeMbps))

            // Phase 2: Copy to SAF destination (sequential streaming)
            val copyStart = System.nanoTime()
            resolver.openFileDescriptor(contentUri, "w")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).use { fos ->
                    tempFile.inputStream().use { input ->
                        val buffer = ByteArray(1024 * 1024)  // 1MB copy buffer
                        var copied = 0L
                        while (copied < totalSize) {
                            val read = input.read(buffer)
                            if (read == -1) break
                            fos.write(buffer, 0, read)
                            copied += read
                        }
                    }
                }
            }
            val copyElapsed = System.nanoTime() - copyStart

            val copyMbps = (totalSize / (1024.0 * 1024.0)) / (copyElapsed / 1_000_000_000.0)
            Log.i(TAG, "  Phase 2 (copy to SAF): %.1f MB/s".format(copyMbps))

            val totalElapsed = writeElapsed + copyElapsed

            logResult("realSaf_flud_strategy_1MB", totalSize, totalElapsed)

            // Also log the breakdown
            Log.i(TAG, "  Total time: write=%.0fms + copy=%.0fms = %.0fms".format(
                writeElapsed / 1_000_000.0,
                copyElapsed / 1_000_000.0,
                totalElapsed / 1_000_000.0
            ))
        } finally {
            tempFile.delete()
            try {
                resolver.delete(contentUri, null, null)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete test file", e)
            }
        }
    }

    /**
     * 4 workers writing to MediaStore content:// via mmap.
     * Tests parallel SAF write performance.
     */
    @Test
    fun realSaf_mediaStore_4workers_mmap() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val numWorkers = 4
        val totalPieces = (TOTAL_DATA_MB * 1024 * 1024) / pieceSize
        val piecesPerWorker = totalPieces / numWorkers
        val totalSize = totalPieces.toLong() * pieceSize
        val data = randomData(pieceSize)

        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "throughput_4w_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri == null) {
            Log.e(TAG, "REAL SAF 4W: Failed to create MediaStore entry")
            return
        }

        Log.i(TAG, "REAL SAF 4-WORKER MMAP: MediaStore, ${pieceSize/1024}KB pieces, $totalPieces total pieces")

        try {
            // Pre-allocate
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                    channel.truncate(totalSize)
                    channel.position(totalSize - 1)
                    channel.write(ByteBuffer.wrap(byteArrayOf(0)))
                }
            }

            val allOffsets = (0 until totalPieces).map { it.toLong() * pieceSize }.shuffled()

            val executor = Executors.newFixedThreadPool(numWorkers)
            val latch = CountDownLatch(numWorkers)
            val totalBytes = AtomicLong(0)

            val start = System.nanoTime()

            // Open mmap once, share across workers
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                val address = android.system.Os.mmap(
                    0,
                    totalSize,
                    android.system.OsConstants.PROT_READ or android.system.OsConstants.PROT_WRITE,
                    android.system.OsConstants.MAP_SHARED,
                    pfd.fileDescriptor,
                    0
                )

                try {
                    for (w in 0 until numWorkers) {
                        val workerOffsets = allOffsets.subList(w * piecesPerWorker, (w + 1) * piecesPerWorker)
                        executor.submit {
                            try {
                                for (offset in workerOffsets) {
                                    MmapHelper.copyToAddress(address + offset, data, 0, data.size)
                                    totalBytes.addAndGet(pieceSize.toLong())
                                }
                            } finally {
                                latch.countDown()
                            }
                        }
                    }
                    latch.await()
                } finally {
                    android.system.Os.munmap(address, totalSize)
                }
            }

            val elapsed = System.nanoTime() - start
            executor.shutdown()

            logResult("realSaf_4workers_mmap_1MB", totalBytes.get(), elapsed)
        } finally {
            try {
                resolver.delete(contentUri, null, null)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete test file", e)
            }
        }
    }

    /**
     * Compare: Direct write to file:// vs content:// for same size.
     * Side-by-side comparison to quantify SAF overhead.
     */
    @Test
    fun comparison_fileVsSaf() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val totalMB = TOTAL_DATA_MB
        val totalSize = totalMB.toLong() * 1024 * 1024
        val numPieces = totalSize.toInt() / pieceSize
        val data = randomData(pieceSize)
        val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

        Log.i(TAG, "COMPARISON: file:// vs content:// random writes, ${pieceSize/1024}KB pieces, ${totalMB}MB")
        Log.i(TAG, "=" .repeat(60))

        // Test 1: file:// with RandomAccessFile
        val nativeFile = File(testDir, "compare_native.bin")
        RandomAccessFile(nativeFile, "rw").use { it.setLength(totalSize) }

        val nativeStart = System.nanoTime()
        RandomAccessFile(nativeFile, "rw").use { raf ->
            for (offset in offsets) {
                raf.seek(offset)
                raf.write(data)
            }
        }
        val nativeElapsed = System.nanoTime() - nativeStart
        val nativeMbps = (totalSize / (1024.0 * 1024.0)) / (nativeElapsed / 1_000_000_000.0)
        Log.i(TAG, "file:// RandomAccessFile: %.1f MB/s".format(nativeMbps))

        // Test 2: file:// with mmap
        val mmapFile = File(testDir, "compare_mmap.bin")
        RandomAccessFile(mmapFile, "rw").use { it.setLength(totalSize) }

        val mmapStart = System.nanoTime()
        RandomAccessFile(mmapFile, "rw").use { raf ->
            raf.channel.use { channel ->
                val buffer = channel.map(FileChannel.MapMode.READ_WRITE, 0, totalSize)
                for (offset in offsets) {
                    val dup = buffer.duplicate()
                    dup.position(offset.toInt())
                    dup.put(data)
                }
            }
        }
        val mmapElapsed = System.nanoTime() - mmapStart
        val mmapMbps = (totalSize / (1024.0 * 1024.0)) / (mmapElapsed / 1_000_000_000.0)
        Log.i(TAG, "file:// mmap: %.1f MB/s".format(mmapMbps))

        // Test 3: content:// with FileChannel
        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "compare_saf_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri != null) {
            try {
                // Pre-allocate
                resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                    java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                        channel.truncate(totalSize)
                        channel.position(totalSize - 1)
                        channel.write(ByteBuffer.wrap(byteArrayOf(0)))
                    }
                }

                val safStart = System.nanoTime()
                resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                    java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                        for (offset in offsets) {
                            channel.position(offset)
                            channel.write(ByteBuffer.wrap(data))
                        }
                    }
                }
                val safElapsed = System.nanoTime() - safStart
                val safMbps = (totalSize / (1024.0 * 1024.0)) / (safElapsed / 1_000_000_000.0)
                Log.i(TAG, "content:// FileChannel: %.1f MB/s".format(safMbps))

                // Test 4: content:// with Os.mmap
                val safMmapStart = System.nanoTime()
                resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                    val address = android.system.Os.mmap(
                        0, totalSize,
                        android.system.OsConstants.PROT_READ or android.system.OsConstants.PROT_WRITE,
                        android.system.OsConstants.MAP_SHARED,
                        pfd.fileDescriptor, 0
                    )
                    try {
                        for (offset in offsets) {
                            MmapHelper.copyToAddress(address + offset, data, 0, data.size)
                        }
                    } finally {
                        android.system.Os.munmap(address, totalSize)
                    }
                }
                val safMmapElapsed = System.nanoTime() - safMmapStart
                val safMmapMbps = (totalSize / (1024.0 * 1024.0)) / (safMmapElapsed / 1_000_000_000.0)
                Log.i(TAG, "content:// Os.mmap: %.1f MB/s".format(safMmapMbps))

                Log.i(TAG, "-".repeat(60))
                Log.i(TAG, "SUMMARY:")
                Log.i(TAG, "  file:// RAF:      %.1f MB/s (baseline)".format(nativeMbps))
                Log.i(TAG, "  file:// mmap:     %.1f MB/s (%.1fx)".format(mmapMbps, mmapMbps / nativeMbps))
                Log.i(TAG, "  content:// chan:  %.1f MB/s (%.1fx)".format(safMbps, safMbps / nativeMbps))
                Log.i(TAG, "  content:// mmap:  %.1f MB/s (%.1fx)".format(safMmapMbps, safMmapMbps / nativeMbps))

            } finally {
                try {
                    resolver.delete(contentUri, null, null)
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to delete test file", e)
                }
            }
        }

        logResult("comparison_file_raf", totalSize, nativeElapsed)
    }

    /**
     * Stress test: 500MB write to content:// via mmap.
     * Tests sustained SAF write throughput past page cache.
     */
    @Test
    fun realSaf_stress_500MB_mmap() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) {
            Log.i(TAG, "SKIP: MediaStore API requires Android 10+")
            return
        }

        val pieceSize = PIECE_1MB
        val totalMB = 500
        val totalSize = totalMB.toLong() * 1024 * 1024
        val numPieces = totalSize.toInt() / pieceSize
        val data = randomData(pieceSize)

        val contentValues = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Downloads.DISPLAY_NAME, "stress_500mb_${System.currentTimeMillis()}.bin")
            put(android.provider.MediaStore.Downloads.MIME_TYPE, "application/octet-stream")
            put(android.provider.MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = context.contentResolver
        val collection = android.provider.MediaStore.Downloads.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val contentUri = resolver.insert(collection, contentValues)

        if (contentUri == null) {
            Log.e(TAG, "STRESS SAF 500MB: Failed to create MediaStore entry")
            return
        }

        Log.i(TAG, "STRESS SAF: 500MB via content:// mmap, ${pieceSize/1024}KB pieces")

        try {
            // Pre-allocate
            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                java.io.FileOutputStream(pfd.fileDescriptor).channel.use { channel ->
                    channel.truncate(totalSize)
                    channel.position(totalSize - 1)
                    channel.write(ByteBuffer.wrap(byteArrayOf(0)))
                }
            }

            val offsets = (0 until numPieces).map { it.toLong() * pieceSize }.shuffled()

            val start = System.nanoTime()
            var bytesWritten = 0L

            resolver.openFileDescriptor(contentUri, "rw")?.use { pfd ->
                val address = android.system.Os.mmap(
                    0, totalSize,
                    android.system.OsConstants.PROT_READ or android.system.OsConstants.PROT_WRITE,
                    android.system.OsConstants.MAP_SHARED,
                    pfd.fileDescriptor, 0
                )

                try {
                    for ((idx, offset) in offsets.withIndex()) {
                        MmapHelper.copyToAddress(address + offset, data, 0, data.size)
                        bytesWritten += pieceSize

                        // Log progress every 100MB
                        if (bytesWritten % (100 * 1024 * 1024) == 0L) {
                            val now = System.nanoTime()
                            val elapsedSec = (now - start) / 1_000_000_000.0
                            val mbps = (bytesWritten / (1024.0 * 1024.0)) / elapsedSec
                            Log.i(TAG, "  Progress: ${bytesWritten / (1024*1024)}MB written, %.1f MB/s avg".format(mbps))
                        }
                    }
                } finally {
                    android.system.Os.munmap(address, totalSize)
                }
            }
            val elapsed = System.nanoTime() - start

            logResult("realSaf_stress_500MB_mmap", bytesWritten, elapsed)
        } finally {
            try {
                resolver.delete(contentUri, null, null)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to delete test file", e)
            }
        }
    }

    // =========================================================================
    // HELPERS
    // =========================================================================

    private fun randomData(size: Int): ByteArray {
        return ByteArray(size).also { Random.nextBytes(it) }
    }

    private fun logResult(testName: String, totalBytes: Long, elapsedNanos: Long) {
        val elapsedMs = elapsedNanos / 1_000_000
        val mbWritten = totalBytes / (1024.0 * 1024.0)
        val mbPerSec = if (elapsedMs > 0) mbWritten / (elapsedMs / 1000.0) else 0.0

        Log.i(TAG, "RESULT [$testName]: %.1f MB in %d ms = %.1f MB/s".format(
            mbWritten, elapsedMs, mbPerSec))
        Log.i(TAG, "-".repeat(60))
    }
}
