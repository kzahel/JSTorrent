# Android Memory Performance Tuning

**Status:** Authoritative reference
**Date:** 2026-03-06
**Related:** [android-memory-measurement-plan.md](android-memory-measurement-plan.md)

## Purpose

This document captures the Android memory and throughput work that is grounded
in real device measurements.

It records:

1. The fixes already made for Android standalone.
2. The next tuning steps for Android companion / ChromeOS.

## What We Reproduced

### Android standalone

On real Android devices, high-throughput torrents with large piece sizes could
fail in several related ways:

- QuickJS/JNI aborts in binary callback paths
- JVM `OutOfMemoryError` in SAF write paths
- runaway native/JVM/JS memory during foreground or background downloads

The key reproduction used `16 MiB` pieces. A count-only active-piece limit was
not sufficient. Even a small number of active pieces could retain too much
memory.

### Android companion / ChromeOS

The companion path is different:

- engine runs in the extension/UI environment
- network I/O flows over the companion WebSocket
- verified writes are sent to Android over HTTP batch writes
- Android persistence still uses the shared `io-core` file manager

So companion mode avoids QuickJS-in-app issues, but it can still hit similar
memory pressure if too much verified write data is in flight or queued for
Android persistence.

## Fixes Landed For Android Standalone

### 1. Measurement and visibility

Added:

- Android process/JVM/native memory snapshots
- QuickJS memory usage exposure
- engine-side piece/buffer/peer counters
- periodic memory logs
- `onTrimMemory()` visibility
- debug `memory` command and real-device capture harness

### 2. Reduced verified-write copy overhead

Removed unnecessary extra copying in the verified-write path before Android
persistence.

This lowers JVM pressure and transient memory churn without changing behavior.

### 3. Android active-piece byte budget

The engine now applies an Android-specific active-piece memory budget instead of
relying only on piece count.

Key idea:

- limit by bytes, not only by number of active pieces
- `16 MiB` pieces must produce a much lower effective active-piece cap than
  `1 MiB` pieces

### 4. Android verified-write queue backpressure

Standalone now pauses new requests when verified-write backlog grows too large.

Current policy:

- high water: `32 MiB`
- low water: `16 MiB`

This is closer to libtorrent’s model:

- bounded bytes waiting on disk
- stop reading / stop requesting when disk backlog is too large

### 5. SAF positioned fd I/O (`pwrite` / `pread`)

The SAF write path now prefers positioned fd I/O instead of
`FileChannel.write(heap ByteBuffer, position)`.

That removed the fatal large-write OOM path caused by temporary direct-buffer
allocation in the Java/NIO stack.

## Current Standalone Outcome

After the fixes above:

- the previously reproduced fatal foreground crash no longer reproduced
- a backgrounded Pixel 9 run completed its capture interval without process
  death
- memory stayed bounded instead of exploding

That means the first critical crash path was fixed. It does not mean Android
memory tuning is finished.

## Companion / ChromeOS Gaps

### 1. Companion write backlog is not byte-budgeted enough

In companion mode, verified writes can remain in flight while waiting for HTTP
completion and WebSocket ACKs.

If this is tracked only by count, large-piece torrents can still retain too much
memory.

### 2. Adaptive batch concurrency was effectively unbounded

If multiple large batch bodies are built concurrently, ChromeOS companion mode
can recreate the same kind of pressure problem through the browser/extension
side, even if Android itself is streaming the HTTP body.

### 3. Android companion worker queue is count-bounded, not byte-bounded

The streaming server currently bounds queued writes by job count. That is not a
safe memory model for large pieces.

### 4. ChromeOS still lacks its own piece-memory profile

`chromeos` should likely be more conservative than desktop, even if it is less
constrained than Android standalone.

## Companion Changes To Apply First

These are the highest-value next steps for companion mode:

1. Add byte-based companion write queue stats.
2. Wire those stats into `BtEngine` backpressure on ChromeOS.
3. Replace unbounded adaptive batch concurrency with an in-flight batch-byte
   budget.

These are the right first changes because they:

- align with the standalone fix strategy
- are low-risk for desktop
- improve memory behavior without changing core torrent semantics

## Follow-Up Companion Work

After the first three changes above, the next likely work items are:

1. Byte-budget the Android companion server queue itself.
2. Consider a ChromeOS active-piece byte budget.
3. Add companion-specific memory and queue visibility to the debug surface.
4. Compare queue thresholds against measured Chromebook workloads.

## Shelved Ideas

These are plausible follow-ups, but not current priorities.

### Stream companion batch uploads from the extension

Today the companion client builds a full packed batch buffer in JS before
sending it with `fetch()`.

Future direction:

- stream the verified-write batch body from JS instead of materializing one
  contiguous packed buffer first
- this would reduce temporary extension-side batch allocations
- it would require a transport/protocol update on the companion server side,
  because the current implementation expects a fixed `Content-Length`

### Use a small bounded staging-buffer pool on the extension side

If streaming is deferred, another option is to bound temporary allocation churn
with a small reusable pool of staging buffers when building companion batch
bodies.

Notes:

- this may or may not matter much in practice on V8
- V8 may already make this less painful than it looks
- if pursued, the pool should be small and explicitly byte-bounded
- this is less attractive than true streaming, but simpler to prototype

## Libtorrent Comparison

The useful libtorrent principle here is not “bigger cache.”

It is:

- bound bytes waiting on disk
- apply backpressure when disk falls behind
- use hysteresis so reads resume only after backlog drains

That principle now exists in Android standalone and should be mirrored more
closely in companion mode.
