@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package com.jstorrent.app.player

import android.net.Uri
import android.util.Log
import androidx.media3.common.C
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import com.jstorrent.app.JSTorrentApplication
import java.io.IOException
import kotlin.math.min
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking

/**
 * Media3 DataSource backed by a torrent file playback session in QuickJS.
 *
 * Media3 expects blocking reads here, so the suspend torrent bridge is adapted
 * with runBlocking on the player loader thread.
 */
class TorrentPlaybackDataSource(
    private val app: JSTorrentApplication,
    private val request: PlayerLaunchRequest
) : BaseDataSource(false) {

    companion object {
        private const val TAG = "TorrentPlaybackDS"
        private const val MIN_FETCH_BYTES = 256 * 1024
    }

    private var currentUri: Uri? = null
    private var byteSource: EnginePlaybackByteSource? = null
    private var opened = false
    private var readPosition = 0L
    private var bytesRemaining = 0L
    private var cachedChunkStart = 0L
    private var cachedChunk = ByteArray(0)

    override fun open(dataSpec: DataSpec): Long {
        try {
            check(!opened) { "DataSource already opened" }

            transferInitializing(dataSpec)
            currentUri = dataSpec.uri

            val source = runBlocking(Dispatchers.IO) {
                val controller = app.ensureEngineStarted()
                EnginePlaybackByteSource.open(controller, request.infoHash, request.fileIndex)
            }

            val fileSize = source.fileSize
            val startPosition = dataSpec.position
            require(startPosition >= 0) { "Negative playback position: $startPosition" }
            require(startPosition <= fileSize) {
                "Playback position $startPosition beyond file size $fileSize"
            }

            val requestedLength = dataSpec.length
            val resolvedLength = if (requestedLength == C.LENGTH_UNSET.toLong()) {
                fileSize - startPosition
            } else {
                min(requestedLength, fileSize - startPosition)
            }

            Log.i(
                TAG,
                "open uri=${dataSpec.uri} position=$startPosition length=$requestedLength resolved=$resolvedLength fileSize=$fileSize"
            )

            byteSource = source
            readPosition = startPosition
            bytesRemaining = resolvedLength
            cachedChunkStart = startPosition
            cachedChunk = ByteArray(0)
            opened = true
            transferStarted(dataSpec)
            return resolvedLength
        } catch (t: Throwable) {
            Log.e(
                TAG,
                "open failed uri=${dataSpec.uri} position=${dataSpec.position} length=${dataSpec.length}: ${t.message}",
                t
            )
            throw IOException("Torrent playback open failed: ${t.message}", t)
        }
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        try {
            if (length == 0) return 0
            if (bytesRemaining == 0L) return C.RESULT_END_OF_INPUT

            val source = byteSource ?: return C.RESULT_END_OF_INPUT
            val bytesToRead = min(length.toLong(), bytesRemaining).toInt()
            val start = readPosition

            val cachedBytes = tryReadFromCache(buffer, offset, bytesToRead)
            if (cachedBytes > 0) {
                Log.d(TAG, "read cache-hit position=$start requested=$bytesToRead served=$cachedBytes")
                return cachedBytes
            }

            val chunk = refillCache(source, bytesToRead)
            if (chunk.isEmpty()) {
                Log.i(TAG, "read eof position=$start requested=$bytesToRead")
                bytesRemaining = 0L
                return C.RESULT_END_OF_INPUT
            }

            val copied = tryReadFromCache(buffer, offset, bytesToRead)
            if (copied > 0) {
                return copied
            }

            throw IOException("Cache refill produced no readable bytes at $start")
        } catch (t: Throwable) {
            Log.e(
                TAG,
                "read failed position=$readPosition requested=$length remaining=$bytesRemaining: ${t.message}",
                t
            )
            throw IOException("Torrent playback read failed at $readPosition: ${t.message}", t)
        }
    }

    override fun getUri(): Uri? = currentUri

    override fun close() {
        Log.i(TAG, "close uri=$currentUri position=$readPosition remaining=$bytesRemaining")
        byteSource?.close()
        byteSource = null
        currentUri = null
        readPosition = 0L
        bytesRemaining = 0L
        cachedChunkStart = 0L
        cachedChunk = ByteArray(0)

        if (opened) {
            opened = false
            transferEnded()
        }
    }

    private fun tryReadFromCache(target: ByteArray, targetOffset: Int, requestedLength: Int): Int {
        if (cachedChunk.isEmpty()) return 0

        val cacheStart = cachedChunkStart
        val cacheEnd = cacheStart + cachedChunk.size
        if (readPosition < cacheStart || readPosition >= cacheEnd) return 0

        val chunkOffset = (readPosition - cacheStart).toInt()
        val available = cachedChunk.size - chunkOffset
        val bytesToCopy = min(requestedLength, available)
        System.arraycopy(cachedChunk, chunkOffset, target, targetOffset, bytesToCopy)
        readPosition += bytesToCopy
        bytesRemaining -= bytesToCopy
        bytesTransferred(bytesToCopy)
        return bytesToCopy
    }

    private fun refillCache(source: EnginePlaybackByteSource, requestedLength: Int): ByteArray {
        val fetchLength = min(
            maxOf(requestedLength, MIN_FETCH_BYTES).toLong(),
            bytesRemaining
        ).toInt()
        val start = readPosition
        val chunk = runBlocking(Dispatchers.IO) {
            source.read(start, fetchLength)
        }
        cachedChunkStart = start
        cachedChunk = chunk
        Log.d(
            TAG,
            "refill cache start=$start requested=$requestedLength fetched=${chunk.size} targetFetch=$fetchLength"
        )
        return chunk
    }
}

class TorrentPlaybackDataSourceFactory(
    private val app: JSTorrentApplication,
    private val request: PlayerLaunchRequest
) : DataSource.Factory {
    override fun createDataSource(): DataSource {
        return TorrentPlaybackDataSource(app, request)
    }
}
