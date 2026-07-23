# Bug: SAF Duplicate Directory Race Condition

## Status: Fixed

## Symptom

When deleting torrents with "remove data" on ChromeOS (extension + Android companion), the engine gets 404 errors from `POST /ops/delete`. The behavior is inconsistent: sometimes deletion works, sometimes it doesn't. On the filesystem, duplicate directories appear with suffixed names like `Big Buck Bunny (1)` and `Big Buck Bunny (2)` instead of a single `Big Buck Bunny`.

Files end up written into the wrong directory, so when the engine later tries to delete `Big Buck Bunny/movie.mp4`, the companion can't find it — the actual file is at `Big Buck Bunny (1)/movie.mp4`.

## Root Cause

Android's Storage Access Framework (SAF) `DocumentFile.createDirectory(name)` does **not** behave like POSIX `mkdir`. On a POSIX filesystem, `mkdir` on an existing directory either succeeds silently or returns EEXIST. SAF's `createDirectory` **always creates a new directory**. If a directory with that display name already exists, SAF creates one with a deduplicated name like `name (1)`.

`FileManagerImpl` correctly guards against this with a `findFile` check before `createDirectory`:

```kotlin
// FileManagerImpl.kt:541-547 (createFile)
for (segment in dirSegments) {
    val existing = current.findFile(segment)
    current = if (existing != null && existing.isDirectory) {
        existing
    } else {
        current.createDirectory(segment) ?: return null
    }
}
```

The same pattern exists in `mkdir` (lines 299-307).

**The problem is concurrency.** The `findFile` + `createDirectory` sequence is not atomic. When two threads both need to create the same directory:

```
Thread A: findFile("Big Buck Bunny") → null
Thread B: findFile("Big Buck Bunny") → null
Thread A: createDirectory("Big Buck Bunny") → creates "Big Buck Bunny"
Thread B: createDirectory("Big Buck Bunny") → creates "Big Buck Bunny (1)"
```

Thread B's files then get written into the wrong directory.

## Affected Code Paths

### 1. Companion mode (extension + Android app on ChromeOS)

The Netty HTTP server handles write requests on worker threads. When the extension downloads a multi-file torrent, it sends concurrent `POST /write/:rootKey` requests for different files. Each request calls `FileManagerImpl.write()` → `getPooledSafHandle()` → `createFile()`, which creates parent directories.

**Call chain:**
```
Extension engine (browser)
  → HTTP POST /write/:rootKey (concurrent requests for different files)
    → NettyHttpServer.handleWrite() [Netty worker thread]
      → fileManager.write(rootUri, relativePath, offset, data)
        → getPooledSafHandle(rootUri, relativePath)
          → createFile(rootUri, relativePath)
            → findFile(segment) + createDirectory(segment)  ← RACE HERE
```

### 2. Standalone mode (Android app running its own engine)

The `NativeBatchingDiskQueue` sends batched writes via JNI. `FileBindings.kt` unpacks the batch and launches each write **in parallel** on `Dispatchers.IO`:

```kotlin
// FileBindings.kt:881-890
// Launch all writes in parallel on I/O dispatcher
for (write in writes) {
    ioScope.launch {
        fileManager.write(rootUri, write.path, write.position, write.data)
    }
}
```

This hits the same `FileManagerImpl.write()` → `getPooledSafHandle()` → `createFile()` path, with the same race condition.

### Why the existing `creationLocks` don't help

`FileManagerImpl` already has per-path locks (line 175):

```kotlin
private val creationLocks = ConcurrentHashMap<String, ReentrantLock>()
```

These are used in `getPooledSafHandle()` (line 603) and are keyed by the **full file path**, e.g., `rootUri|Big Buck Bunny/movie.mp4`. Two different files in the same directory use different lock keys:

- `rootUri|Big Buck Bunny/movie.mp4` — Lock A
- `rootUri|Big Buck Bunny/subs.srt` — Lock B

Both threads acquire their own lock and proceed to create the `Big Buck Bunny` directory concurrently. The locks protect against two threads creating the same *file*, but not against two threads creating the same *directory*.

### Why it's inconsistent

The race depends on timing. If one thread completes `createDirectory` before the other thread calls `findFile`, the second thread finds the existing directory and uses it — no duplicate. This is why it sometimes works and sometimes doesn't, depending on thread scheduling.

## Downstream Effects

1. **Files written to wrong directory** — Content ends up in `Big Buck Bunny (1)/` instead of `Big Buck Bunny/`
2. **Delete fails with 404** — Engine tries to delete `Big Buck Bunny/movie.mp4`, but the file is actually at `Big Buck Bunny (1)/movie.mp4`. The companion's `/ops/delete` handler returns 404 because `resolvePath` can't find the file at the expected path.
3. **Orphaned data on disk** — The duplicate directories and their contents are never cleaned up since the engine doesn't know about them.
4. **Exists check returns false** — The engine's `exists` → `delete` guard correctly skips deletion when the file isn't at the expected path, but the user's data is still on disk under the wrong name.

## Recommended Fix: Per-Directory Lock in FileManagerImpl

Add a `ConcurrentHashMap<String, ReentrantLock>` keyed by directory path (not file path). Both `createFile` and `mkdir` should acquire this lock before the `findFile` + `createDirectory` sequence for each directory segment.

### Lock key format

`"$rootUri|$dirPath"` where `dirPath` is the full path of the directory being created, e.g., `content://....|Big Buck Bunny` for a top-level torrent directory, or `content://....|Big Buck Bunny/extras` for a subdirectory.

### What to lock

The critical section is the `findFile(segment)` + `createDirectory(segment)` pair. Both `createFile` (line 541-547) and `mkdir` (line 299-307) have this pattern and both need the same lock.

### Suggested approach

Extract a shared method like `findOrCreateDirectory(parent: DocumentFile, segment: String, rootUri: Uri, parentPath: String): DocumentFile?` that:

1. Computes the lock key from `rootUri` and the full directory path
2. Acquires the per-directory lock
3. Calls `parent.findFile(segment)`
4. If found and is a directory, returns it
5. If not found, calls `parent.createDirectory(segment)` and returns the result
6. Releases the lock (via `withLock`)

Both `createFile` and `mkdir` call this helper instead of inlining the `findFile` + `createDirectory` logic.

### Scope of fix

- **Only `FileManagerImpl.kt` needs changes** — The fix is entirely within the SAF path of `FileManagerImpl`
- **Native `file://` path is not affected** — Java's `File.mkdirs()` is idempotent (returns true if directory already exists)
- **No engine-side changes needed** — The fix is transparent to the TypeScript engine
- **Protects both companion and standalone** — Both paths go through the same `FileManagerImpl` methods
- **The existing `creationLocks` for file handles can stay** — They serve a different purpose (preventing duplicate file handle creation for the same file path)

### Lock cleanup

Same pattern as the existing `creationLocks`: remove the lock from the map when no threads are queued on it. Directory creation only happens once per directory per torrent session, so the locks are short-lived.

## Files to Modify

| File | Change |
|------|--------|
| `android/io-core/.../FileManagerImpl.kt` | Add per-directory lock, extract `findOrCreateDirectory` helper, update `createFile` and `mkdir` to use it |

## Verification

1. `./gradlew :io-core:compileDebugKotlin` — Compile check
2. `./gradlew testDebugUnitTest` — Unit tests
3. Manual test: download a multi-file torrent on ChromeOS via companion, verify no duplicate directories appear, verify "remove with data" deletes everything without 404s
4. Manual test: same on Android standalone (phone or ChromeOS standalone mode)
