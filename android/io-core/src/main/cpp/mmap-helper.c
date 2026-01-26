/**
 * JNI helper for wrapping mmap'd memory addresses in DirectByteBuffer.
 *
 * This uses the official JNI NewDirectByteBuffer() API, which is not subject
 * to Android's hidden API restrictions (unlike the reflective approach of
 * calling DirectByteBuffer's constructor).
 *
 * Usage from Kotlin:
 *   val buffer = MmapHelper.wrapAddress(mmapAddress, size)
 *   buffer.put(data)  // writes directly to mmap'd memory
 */

#include <jni.h>
#include <android/log.h>
#include <string.h>
#include <sys/mman.h>

#define LOG_TAG "MmapHelper"
#define LOGD(...) __android_log_print(ANDROID_LOG_DEBUG, LOG_TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, LOG_TAG, __VA_ARGS__)

/**
 * Wrap a native memory address in a DirectByteBuffer.
 *
 * @param address The native memory address (from Os.mmap())
 * @param capacity The size of the mapped region in bytes
 * @return A DirectByteBuffer wrapping the memory, or null on error
 */
JNIEXPORT jobject JNICALL
Java_com_jstorrent_io_file_MmapHelper_nativeWrapAddress(
    JNIEnv *env,
    jclass clazz,
    jlong address,
    jlong capacity
) {
    (void)clazz;

    if (address == 0) {
        LOGE("nativeWrapAddress: null address");
        return NULL;
    }

    if (capacity <= 0 || capacity > 0x7FFFFFFF) {
        // DirectByteBuffer capacity is limited to Integer.MAX_VALUE
        LOGE("nativeWrapAddress: invalid capacity %lld", (long long)capacity);
        return NULL;
    }

    // NewDirectByteBuffer is the official JNI way to create a DirectByteBuffer
    // wrapping native memory. It's not subject to hidden API restrictions.
    jobject buffer = (*env)->NewDirectByteBuffer(env, (void*)address, capacity);

    if (buffer == NULL) {
        LOGE("nativeWrapAddress: NewDirectByteBuffer failed");
        return NULL;
    }

    LOGD("Wrapped mmap address 0x%llx with capacity %lld",
         (unsigned long long)address, (long long)capacity);

    return buffer;
}

/**
 * Copy data directly to a mmap'd address.
 * This avoids creating a ByteBuffer wrapper for simple write operations.
 *
 * @param destAddress The destination mmap address
 * @param data The source byte array
 * @param offset Offset within the source array
 * @param length Number of bytes to copy
 */
JNIEXPORT void JNICALL
Java_com_jstorrent_io_file_MmapHelper_nativeCopyToAddress(
    JNIEnv *env,
    jclass clazz,
    jlong destAddress,
    jbyteArray data,
    jint offset,
    jint length
) {
    (void)clazz;

    if (destAddress == 0 || data == NULL || length <= 0) {
        LOGE("nativeCopyToAddress: invalid args");
        return;
    }

    jbyte *bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (bytes == NULL) {
        LOGE("nativeCopyToAddress: failed to get array elements");
        return;
    }

    // Direct memory copy - very fast
    memcpy((void*)destAddress, bytes + offset, (size_t)length);

    // Release without copying back (JNI_ABORT)
    (*env)->ReleaseByteArrayElements(env, data, bytes, JNI_ABORT);
}

/**
 * Copy data from a mmap'd address to a byte array.
 *
 * @param srcAddress The source mmap address
 * @param data The destination byte array
 * @param offset Offset within the destination array
 * @param length Number of bytes to copy
 */
JNIEXPORT void JNICALL
Java_com_jstorrent_io_file_MmapHelper_nativeCopyFromAddress(
    JNIEnv *env,
    jclass clazz,
    jlong srcAddress,
    jbyteArray data,
    jint offset,
    jint length
) {
    (void)clazz;

    if (srcAddress == 0 || data == NULL || length <= 0) {
        LOGE("nativeCopyFromAddress: invalid args");
        return;
    }

    jbyte *bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (bytes == NULL) {
        LOGE("nativeCopyFromAddress: failed to get array elements");
        return;
    }

    // Direct memory copy
    memcpy(bytes + offset, (void*)srcAddress, (size_t)length);

    // Commit the changes (0 = copy back and free)
    (*env)->ReleaseByteArrayElements(env, data, bytes, 0);
}
