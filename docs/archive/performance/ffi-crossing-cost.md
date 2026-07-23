# FFI Crossing Cost Analysis

## Summary

Measured the cost of Kotlin↔JS FFI crossings on Android to inform architecture decisions about data flow between native and JS layers.

**Key finding: FFI crossings are essentially free (~7 µs), not the 10-12ms we hypothesized.**

## Benchmark Results (Pixel 7a, 2025-01-28)

```
=== FFI RTT Benchmark Results ===
noop:              6.8 µs (0.007 ms)
binary_in_1kb:     9.1 µs (0.009 ms)
binary_in_16kb:   10.1 µs (0.010 ms)
binary_in_64kb:   15.4 µs (0.015 ms)
binary_in_256kb:  46.3 µs (0.046 ms)
binary_echo_1kb:   5.2 µs (0.005 ms)
binary_echo_16kb: 16.3 µs (0.016 ms)
binary_echo_64kb: 40.9 µs (0.041 ms)
binary_echo_256kb: 126.4 µs (0.126 ms)
```

### Test Definitions

- `noop`: Kotlin calls JS function that returns 0 immediately
- `binary_in_*`: Kotlin passes ByteArray to JS, JS returns `data.byteLength`
- `binary_echo_*`: Kotlin passes ByteArray to JS, JS returns the ArrayBuffer back

### Analysis

| Metric | Value |
|--------|-------|
| Base FFI overhead (no data) | ~7 µs |
| Copy overhead | ~0.2 µs per KB |
| 16KB one-way | 10 µs |
| 16KB round-trip | 16 µs |
| 256KB round-trip | 126 µs |

## Implications

### What This Means

1. **FFI is NOT a bottleneck** - At 7 µs per call, we could make 140,000 crossings per second
2. **Data copying is cheap** - 256KB round-trip is only 0.13ms
3. **Multiple crossings per tick are fine** - No need to minimize FFI calls

### What This Rules Out

The 178ms effective RTT observed during downloads is NOT caused by:
- FFI crossing overhead
- Binary data copying between Kotlin and JS

### Where the Latency Actually Comes From

- **Handler.post() scheduling** - Messages queue behind other work
- **Tick batching architecture** - Data waits for next tick to be processed
- **Not FFI** - The crossing itself is sub-millisecond

### Architecture Implications

These optimizations are **NOT worth pursuing** (FFI overhead too small to matter):
- Pushing TCP data into the tick call parameter
- Reducing number of FFI crossings
- Batching more aggressively to reduce crossing count

These optimizations **ARE worth pursuing**:
- Tight loop within handler callback (multiple ticks without re-queuing)
- Reducing Handler message queue latency
- Processing data more frequently (more ticks per second)

## How to Run the Benchmark

```bash
# Ensure app is running and engine is initialized
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd rtt -p com.jstorrent.app

# View results
adb logcat -s JSTorrent-Debug EngineController
```

## Implementation

### JS Side (controller.ts)

```typescript
// No-op for measuring base overhead
globalThis.__jstorrent_noop = (): number => 0

// Receives binary, returns length
globalThis.__jstorrent_noop_binary = (data: ArrayBuffer): number => data.byteLength

// Receives and returns binary (echo)
globalThis.__jstorrent_echo_binary = (data: ArrayBuffer): ArrayBuffer => data
```

### Kotlin Side (EngineController.kt)

```kotlin
fun runRttBenchmark(): Map<String, Double> {
    // Posts benchmark work to JS thread
    // Measures nanoTime for 1000 iterations each
    // Returns results in microseconds
}
```

## Related Files

- `packages/engine/src/adapters/native/controller.ts` - JS benchmark functions
- `android/quickjs-engine/src/main/kotlin/com/jstorrent/quickjs/EngineController.kt` - Kotlin runner
- `android/app/src/main/java/com/jstorrent/app/debug/DebugReceiver.kt` - Debug command
