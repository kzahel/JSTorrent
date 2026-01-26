package com.jstorrent.quickjs.bindings

import android.content.Context
import android.net.Uri
import android.util.Log
import com.jstorrent.io.file.FileManager
import com.jstorrent.io.file.FileManagerException
import com.jstorrent.io.hash.Hasher
import com.jstorrent.quickjs.JsThread
import com.jstorrent.quickjs.QuickJsContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

private const val TAG = "FileBindings"

/**
 * Write result codes for async verified writes.
 */
object WriteResultCode {
    const val SUCCESS = 0
    const val HASH_MISMATCH = 1
    const val IO_ERROR = 2
    const val INVALID_ARGS = 3
}

/**
 * File I/O bindings for QuickJS.
 *
 * Implements stateless file operations using [FileManager]:
 * - __jstorrent_file_read(rootKey, path, offset, length) -> ArrayBuffer
 * - __jstorrent_file_write(rootKey, path, offset, data) -> number (sync)
 * - __jstorrent_file_write_verified(rootKey, path, offset, data, expectedSha1Hex, callbackId) -> void (async)
 * - __jstorrent_file_stat(rootKey, path) -> string | null
 * - __jstorrent_file_mkdir(rootKey, path) -> boolean
 * - __jstorrent_file_exists(rootKey, path) -> boolean
 * - __jstorrent_file_readdir(rootKey, path) -> string (JSON array)
 * - __jstorrent_file_delete(rootKey, path) -> boolean
 *
 * Sync operations block the JS thread. The async write_verified operation runs
 * hashing and I/O on a background thread, posting results back to JS via callback.
 *
 * Root resolution:
 * - Empty or "default" rootKey resolves to app-private downloads directory
 * - Other rootKeys are resolved via [rootResolver] (for SAF URIs)
 */
class FileBindings(
    private val context: Context,
    private val fileManager: FileManager,
    private val rootResolver: (String) -> Uri?,
    private val jsThread: JsThread? = null,
) {
    // Coroutine scope for async I/O operations (hash + write on background thread)
    private val ioScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    companion object {
        // Throughput and latency tracking for backpressure detection
        @Volatile private var bytesWritten = 0L
        @Volatile private var writeCount = 0
        @Volatile private var totalWriteTimeMs = 0L
        @Volatile private var maxWriteLatencyMs = 0L
        @Volatile private var lastLogTime = System.currentTimeMillis()

        // Detailed timing instrumentation for async writes
        @Volatile private var pendingWrites = 0
        @Volatile private var maxPendingWrites = 0
        @Volatile private var totalDispatchDelayNs = 0L  // Time waiting for coroutine to start
        @Volatile private var totalHashTimeNs = 0L
        @Volatile private var totalWriteTimeNs = 0L
        @Volatile private var totalPostTimeNs = 0L  // Time for jsThread.post to complete
        @Volatile private var instrumentedCount = 0
    }

    // App-private downloads directory (fallback when rootKey is empty/"default")
    private val appPrivateDownloads: File by lazy {
        File(context.filesDir, "downloads").also { it.mkdirs() }
    }

    /**
     * Register all file bindings on the given context.
     */
    fun register(ctx: QuickJsContext) {
        registerReadWrite(ctx)
        registerAsyncWrite(ctx)
        registerPathFunctions(ctx)
    }

    /**
     * Resolve rootKey to a Uri.
     * - Empty or "default" -> app-private downloads directory
     * - Otherwise -> use rootResolver (for SAF URIs)
     */
    private fun resolveRoot(rootKey: String): Uri? {
        return when {
            rootKey.isEmpty() || rootKey == "default" ->
                Uri.fromFile(appPrivateDownloads)
            else -> rootResolver(rootKey)
        }
    }

    /**
     * Register stateless read/write functions.
     */
    private fun registerReadWrite(ctx: QuickJsContext) {
        // __jstorrent_file_read(rootKey: string, path: string, offset: number, length: number): ArrayBuffer
        ctx.setGlobalFunctionReturnsBinary("__jstorrent_file_read") { args, _ ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""
            val offset = args.getOrNull(2)?.toLongOrNull() ?: 0L
            val length = args.getOrNull(3)?.toIntOrNull() ?: 0

            if (path.isEmpty() || length <= 0) {
                return@setGlobalFunctionReturnsBinary ByteArray(0)
            }

            val rootUri = resolveRoot(rootKey)
            if (rootUri == null) {
                Log.w(TAG, "Unknown root key: $rootKey")
                return@setGlobalFunctionReturnsBinary ByteArray(0)
            }

            try {
                fileManager.read(rootUri, path, offset, length)
            } catch (e: FileManagerException) {
                Log.e(TAG, "Read failed: $path", e)
                ByteArray(0)
            } catch (e: Exception) {
                Log.e(TAG, "Read failed: $path", e)
                ByteArray(0)
            }
        }

        // __jstorrent_file_write(rootKey: string, path: string, offset: number, data: ArrayBuffer): number
        ctx.setGlobalFunctionWithBinary("__jstorrent_file_write", 3) { args, binary ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""
            val offset = args.getOrNull(2)?.toLongOrNull() ?: 0L

            if (path.isEmpty() || binary == null) {
                return@setGlobalFunctionWithBinary "-1"
            }

            val rootUri = resolveRoot(rootKey)
            if (rootUri == null) {
                Log.w(TAG, "Unknown root key: $rootKey")
                return@setGlobalFunctionWithBinary "-1"
            }

            try {
                val startTime = System.currentTimeMillis()
                fileManager.write(rootUri, path, offset, binary)
                val elapsed = System.currentTimeMillis() - startTime

                // Track stats
                bytesWritten += binary.size
                writeCount++
                totalWriteTimeMs += elapsed
                if (elapsed > maxWriteLatencyMs) {
                    maxWriteLatencyMs = elapsed
                }

                // Log every 5 seconds
                val now = System.currentTimeMillis()
                val sinceLastLog = now - lastLogTime
                if (sinceLastLog >= 5000) {
                    val mbWritten = bytesWritten / (1024.0 * 1024.0)
                    val mbps = mbWritten / (sinceLastLog / 1000.0)
                    val avgLatency = if (writeCount > 0) totalWriteTimeMs / writeCount else 0
                    Log.i(TAG, "Disk write: %.2f MB/s, %d writes, avg %dms, max %dms".format(
                        mbps, writeCount, avgLatency, maxWriteLatencyMs))
                    bytesWritten = 0
                    writeCount = 0
                    totalWriteTimeMs = 0
                    maxWriteLatencyMs = 0
                    lastLogTime = now
                }

                binary.size.toString()
            } catch (e: FileManagerException) {
                Log.e(TAG, "Write failed: $path", e)
                "-1"
            } catch (e: Exception) {
                Log.e(TAG, "Write failed: $path", e)
                "-1"
            }
        }
    }

    /**
     * Register functions that operate on paths.
     */
    private fun registerPathFunctions(ctx: QuickJsContext) {
        // __jstorrent_file_preallocate(rootKey: string, path: string, size: number): boolean
        // Pre-allocate file for faster writes (enables memory-mapped I/O)
        ctx.setGlobalFunction("__jstorrent_file_preallocate") { args ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""
            val size = args.getOrNull(2)?.toLongOrNull() ?: 0L

            if (path.isEmpty() || size <= 0) {
                return@setGlobalFunction "false"
            }

            val rootUri = resolveRoot(rootKey) ?: return@setGlobalFunction "false"

            try {
                fileManager.preallocate(rootUri, path, size).toString()
            } catch (e: Exception) {
                Log.e(TAG, "Preallocate failed: $path", e)
                "false"
            }
        }

        // __jstorrent_file_stat(rootKey: string, path: string): string | null
        ctx.setGlobalFunction("__jstorrent_file_stat") { args ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""

            val rootUri = resolveRoot(rootKey) ?: return@setGlobalFunction null

            try {
                val stat = fileManager.stat(rootUri, path) ?: return@setGlobalFunction null
                JSONObject().apply {
                    put("size", stat.size)
                    put("mtime", stat.mtime)
                    put("isDirectory", stat.isDirectory)
                    put("isFile", stat.isFile)
                }.toString()
            } catch (e: Exception) {
                Log.e(TAG, "Stat failed: $path", e)
                null
            }
        }

        // __jstorrent_file_mkdir(rootKey: string, path: string): boolean
        ctx.setGlobalFunction("__jstorrent_file_mkdir") { args ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""

            val rootUri = resolveRoot(rootKey) ?: return@setGlobalFunction "false"

            try {
                fileManager.mkdir(rootUri, path).toString()
            } catch (e: Exception) {
                Log.e(TAG, "Mkdir failed: $path", e)
                "false"
            }
        }

        // __jstorrent_file_exists(rootKey: string, path: string): boolean
        ctx.setGlobalFunction("__jstorrent_file_exists") { args ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""

            val rootUri = resolveRoot(rootKey) ?: return@setGlobalFunction "false"

            try {
                fileManager.exists(rootUri, path).toString()
            } catch (e: Exception) {
                Log.e(TAG, "Exists failed: $path", e)
                "false"
            }
        }

        // __jstorrent_file_readdir(rootKey: string, path: string): string (JSON array)
        ctx.setGlobalFunction("__jstorrent_file_readdir") { args ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""

            val rootUri = resolveRoot(rootKey) ?: return@setGlobalFunction "[]"

            try {
                val entries = fileManager.readdir(rootUri, path)
                JSONArray(entries).toString()
            } catch (e: Exception) {
                Log.e(TAG, "Readdir failed: $path", e)
                "[]"
            }
        }

        // __jstorrent_file_delete(rootKey: string, path: string): boolean
        ctx.setGlobalFunction("__jstorrent_file_delete") { args ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""

            val rootUri = resolveRoot(rootKey) ?: return@setGlobalFunction "false"

            try {
                fileManager.delete(rootUri, path).toString()
            } catch (e: Exception) {
                Log.e(TAG, "Delete failed: $path", e)
                "false"
            }
        }
    }

    /**
     * Register async verified write function.
     *
     * This moves hashing and I/O to a background thread, freeing the JS thread
     * to continue processing data callbacks. Results are posted back via callback.
     */
    private fun registerAsyncWrite(ctx: QuickJsContext) {
        // Register the JS dispatch function for write results
        ctx.evaluate("""
            globalThis.__jstorrent_file_write_callbacks = {};
            globalThis.__jstorrent_file_dispatch_write_result = function(callbackId, bytesWritten, resultCode) {
                const callback = globalThis.__jstorrent_file_write_callbacks[callbackId];
                if (callback) {
                    delete globalThis.__jstorrent_file_write_callbacks[callbackId];
                    callback(bytesWritten, resultCode);
                }
            };
        """.trimIndent(), "file-bindings-init.js")

        // __jstorrent_file_write_verified(rootKey, path, offset, data, expectedSha1Hex, callbackId): void
        // Async verified write - hashes data, compares to expected, writes if match.
        // Posts result back to JS via __jstorrent_file_dispatch_write_result.
        ctx.setGlobalFunctionWithBinary("__jstorrent_file_write_verified", 3) { args, binary ->
            val rootKey = args.getOrNull(0) ?: ""
            val path = args.getOrNull(1) ?: ""
            val offset = args.getOrNull(2)?.toLongOrNull() ?: 0L
            // arg[3] is binary (data)
            val expectedSha1Hex = args.getOrNull(4) ?: ""
            val callbackId = args.getOrNull(5) ?: ""

            if (jsThread == null) {
                Log.e(TAG, "write_verified: jsThread not available")
                return@setGlobalFunctionWithBinary null
            }

            if (path.isEmpty() || binary == null || expectedSha1Hex.isEmpty() || callbackId.isEmpty()) {
                Log.w(TAG, "write_verified: invalid args")
                // Post error back immediately
                jsThread.post {
                    ctx.callGlobalFunction(
                        "__jstorrent_file_dispatch_write_result",
                        callbackId,
                        "-1",
                        WriteResultCode.INVALID_ARGS.toString()
                    )
                    jsThread.scheduleJobPump(ctx)
                }
                return@setGlobalFunctionWithBinary null
            }

            val rootUri = resolveRoot(rootKey)
            if (rootUri == null) {
                Log.w(TAG, "write_verified: unknown root key: $rootKey")
                jsThread.post {
                    ctx.callGlobalFunction(
                        "__jstorrent_file_dispatch_write_result",
                        callbackId,
                        "-1",
                        WriteResultCode.INVALID_ARGS.toString()
                    )
                    jsThread.scheduleJobPump(ctx)
                }
                return@setGlobalFunctionWithBinary null
            }

            // Track pending writes for backpressure detection
            val queuedAtNs = System.nanoTime()
            synchronized(Companion) {
                pendingWrites++
                if (pendingWrites > maxPendingWrites) {
                    maxPendingWrites = pendingWrites
                }
            }

            // Launch async work on I/O dispatcher
            ioScope.launch {
                val dispatchedAtNs = System.nanoTime()
                val dispatchDelayNs = dispatchedAtNs - queuedAtNs

                try {
                    // 1. Hash the data
                    val hashStartNs = System.nanoTime()
                    val actualHash = Hasher.sha1(binary)
                    val actualHashHex = actualHash.joinToString("") { "%02x".format(it) }
                    val hashTimeNs = System.nanoTime() - hashStartNs

                    // 2. Compare hashes
                    if (!actualHashHex.equals(expectedSha1Hex, ignoreCase = true)) {
                        Log.w(TAG, "write_verified: hash mismatch for $path")
                        synchronized(Companion) { pendingWrites-- }
                        jsThread.post {
                            ctx.callGlobalFunction(
                                "__jstorrent_file_dispatch_write_result",
                                callbackId,
                                "-1",
                                WriteResultCode.HASH_MISMATCH.toString()
                            )
                            jsThread.scheduleJobPump(ctx)
                        }
                        return@launch
                    }

                    // 3. Write the data (hash matched)
                    val writeStartNs = System.nanoTime()
                    fileManager.write(rootUri, path, offset, binary)
                    val writeTimeNs = System.nanoTime() - writeStartNs

                    // 4. Post success back to JS thread
                    val postStartNs = System.nanoTime()
                    jsThread.post {
                        ctx.callGlobalFunction(
                            "__jstorrent_file_dispatch_write_result",
                            callbackId,
                            binary.size.toString(),
                            WriteResultCode.SUCCESS.toString()
                        )
                        jsThread.scheduleJobPump(ctx)
                    }
                    val postTimeNs = System.nanoTime() - postStartNs

                    val totalElapsedMs = (System.nanoTime() - queuedAtNs) / 1_000_000

                    // Track detailed stats
                    synchronized(Companion) {
                        pendingWrites--
                        bytesWritten += binary.size
                        writeCount++
                        totalWriteTimeMs += totalElapsedMs
                        if (totalElapsedMs > maxWriteLatencyMs) {
                            maxWriteLatencyMs = totalElapsedMs
                        }

                        // Accumulate detailed timing
                        totalDispatchDelayNs += dispatchDelayNs
                        totalHashTimeNs += hashTimeNs
                        totalWriteTimeNs += writeTimeNs
                        totalPostTimeNs += postTimeNs
                        instrumentedCount++

                        // Log every 5 seconds with detailed breakdown
                        val now = System.currentTimeMillis()
                        val sinceLastLog = now - lastLogTime
                        if (sinceLastLog >= 5000 && instrumentedCount > 0) {
                            val mbWritten = bytesWritten / (1024.0 * 1024.0)
                            val mbps = mbWritten / (sinceLastLog / 1000.0)
                            val avgTotalMs = if (writeCount > 0) totalWriteTimeMs / writeCount else 0

                            // Convert ns totals to ms averages
                            val avgDispatchMs = (totalDispatchDelayNs / instrumentedCount) / 1_000_000.0
                            val avgHashMs = (totalHashTimeNs / instrumentedCount) / 1_000_000.0
                            val avgWriteMs = (totalWriteTimeNs / instrumentedCount) / 1_000_000.0
                            val avgPostMs = (totalPostTimeNs / instrumentedCount) / 1_000_000.0

                            Log.i(TAG, "Verified write: %.2f MB/s, %d writes, pending=%d (max=%d)".format(
                                mbps, writeCount, pendingWrites, maxPendingWrites))
                            Log.i(TAG, "  Timing breakdown: total=%dms, dispatch=%.1fms, hash=%.1fms, write=%.1fms, post=%.1fms".format(
                                avgTotalMs, avgDispatchMs, avgHashMs, avgWriteMs, avgPostMs))

                            // Reset counters
                            bytesWritten = 0
                            writeCount = 0
                            totalWriteTimeMs = 0
                            maxWriteLatencyMs = 0
                            totalDispatchDelayNs = 0
                            totalHashTimeNs = 0
                            totalWriteTimeNs = 0
                            totalPostTimeNs = 0
                            instrumentedCount = 0
                            maxPendingWrites = pendingWrites
                            lastLogTime = now
                        }
                    }

                } catch (e: Exception) {
                    Log.e(TAG, "write_verified failed: $path", e)
                    synchronized(Companion) { pendingWrites-- }
                    jsThread.post {
                        ctx.callGlobalFunction(
                            "__jstorrent_file_dispatch_write_result",
                            callbackId,
                            "-1",
                            WriteResultCode.IO_ERROR.toString()
                        )
                        jsThread.scheduleJobPump(ctx)
                    }
                }
            }

            null // Return immediately, result comes via callback
        }
    }
}
