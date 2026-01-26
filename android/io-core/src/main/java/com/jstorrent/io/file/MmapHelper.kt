package com.jstorrent.io.file

import android.util.Log
import java.nio.ByteBuffer

/**
 * JNI helper for working with mmap'd memory addresses.
 *
 * This bypasses Android's hidden API restrictions by using the official
 * JNI NewDirectByteBuffer() API instead of reflection to create
 * DirectByteBuffer instances.
 *
 * Usage:
 *   val address = Os.mmap(...)  // Returns Long address
 *   val buffer = MmapHelper.wrapAddress(address, size)
 *   buffer.put(data)  // Writes directly to mmap'd memory
 */
object MmapHelper {
    private const val TAG = "MmapHelper"

    init {
        try {
            System.loadLibrary("io-native")
            Log.i(TAG, "Loaded io-native library")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Failed to load io-native library", e)
        }
    }

    /**
     * Wrap a native mmap address in a DirectByteBuffer.
     *
     * @param address The mmap address from Os.mmap()
     * @param capacity The size of the mapped region (must be <= Int.MAX_VALUE)
     * @return A DirectByteBuffer wrapping the memory, or null on error
     */
    fun wrapAddress(address: Long, capacity: Long): ByteBuffer? {
        if (address == 0L) {
            Log.w(TAG, "Cannot wrap null address")
            return null
        }
        if (capacity <= 0 || capacity > Int.MAX_VALUE) {
            Log.w(TAG, "Invalid capacity: $capacity")
            return null
        }
        return try {
            nativeWrapAddress(address, capacity)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to wrap address", e)
            null
        }
    }

    /**
     * Copy data directly to a mmap'd address.
     * This is faster than wrapping in ByteBuffer for one-shot writes.
     *
     * @param destAddress The destination mmap address
     * @param data Source data
     * @param offset Offset within the source array
     * @param length Number of bytes to copy
     */
    fun copyToAddress(destAddress: Long, data: ByteArray, offset: Int = 0, length: Int = data.size) {
        if (destAddress == 0L || length <= 0) return
        nativeCopyToAddress(destAddress, data, offset, length)
    }

    /**
     * Copy data from a mmap'd address to a byte array.
     *
     * @param srcAddress The source mmap address
     * @param data Destination array
     * @param offset Offset within the destination array
     * @param length Number of bytes to copy
     */
    fun copyFromAddress(srcAddress: Long, data: ByteArray, offset: Int = 0, length: Int = data.size) {
        if (srcAddress == 0L || length <= 0) return
        nativeCopyFromAddress(srcAddress, data, offset, length)
    }

    // Native methods
    @JvmStatic
    private external fun nativeWrapAddress(address: Long, capacity: Long): ByteBuffer?

    @JvmStatic
    private external fun nativeCopyToAddress(destAddress: Long, data: ByteArray, offset: Int, length: Int)

    @JvmStatic
    private external fun nativeCopyFromAddress(srcAddress: Long, data: ByteArray, offset: Int, length: Int)
}
