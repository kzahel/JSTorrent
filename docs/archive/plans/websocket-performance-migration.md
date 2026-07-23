# WebSocket Performance Migration Plan

## Problem Statement

The Kotlin companion server using Ktor WebSocket achieves only **12-14 MB/s** throughput, while:
- Rust daemon (Axum/tungstenite) achieves **45-50 MB/s** on the same ChromeOS hardware
- Android standalone app achieves **45 MB/s** using the same `TcpConnectionNio` code

The bottleneck has been isolated to the **Ktor WebSocket layer**, not:
- TCP socket handling (same `TcpConnectionNio` code path)
- Chrome/ARC communication (raw throughput is 300-400 Mbit)
- Request pipeline depth (500 chunks in flight)
- Disk I/O (tested with null storage)

## Current Test Infrastructure

### Existing Tests (Good Coverage)

| Test File | Type | Coverage |
|-----------|------|----------|
| `ThroughputBenchmarkTest.kt` | JVM Unit | WebSocket throughput with 10MB-100MB transfers, various chunk sizes |
| `WebSocketIOTest.kt` | Instrumented | Handshake, auth flow on real Android |
| `TcpSocketTest.kt` | Instrumented | TCP operations over WebSocket |
| `WebSocketAuthTest.kt` | JVM Unit | Binary protocol AUTH parsing |
| `WebSocketRouteTest.kt` | JVM Unit | Opcode filtering per endpoint |

### Existing Test Utilities

| File | Purpose |
|------|---------|
| `TestWsClient.kt` | Minimal WebSocket client using `java_websocket` library |
| `TestDaemonServer.kt` | Standalone JVM WebSocket server (mimics companion) |
| `benchmark/Protocol.kt` | Binary protocol frame creation/parsing |

### CI Integration

- **Unit tests**: `./gradlew testDebugUnitTest` (runs benchmarks on JVM)
- **Instrumented tests**: `./gradlew connectedDebugAndroidTest` (runs on emulator)

## Architecture Analysis

### Ktor Coupling Points

```
CompanionHttpServer.kt
├── embeddedServer(Netty, port)           ← Server bootstrap
│   ├── install(WebSockets) { ... }       ← Plugin config
│   └── routing {
│       ├── webSocket("/io") { ... }      ← DSL routing
│       └── webSocket("/control") { ... }
│
IoWebSocketHandler.kt
├── wsSession: DefaultWebSocketServerSession  ← Session type
├── for (frame in wsSession.incoming)         ← Frame iteration
├── wsSession.send(Frame.Binary(...))         ← Frame sending
└── ClosedReceiveChannelException             ← Close detection
```

### Already Transport-Agnostic (~80%)

- `Protocol.kt` - Binary protocol parsing
- `TcpSocketService.kt` - TCP connection management
- `TcpConnectionNio.kt` - NIO socket handling
- `SocketManagerFactory.kt` - Socket lifecycle
- Handler business logic (message routing, file I/O)

## Migration Strategy

### Phase 1: Establish Baseline Benchmarks ✅ COMPLETE

**Goal:** Quantify current Ktor performance with reproducible benchmarks.

**Tasks:**
1. [x] Enhance `ThroughputBenchmarkTest.kt` to run against real companion server (not just `TestDaemonServer`)
2. [x] Add throughput logging to match production format (MB/s, frames/s, latency)
3. [x] Create benchmark that measures **sustained** throughput over 30+ seconds
4. [x] Document baseline numbers: Ktor on JVM, Ktor on Android emulator, Ktor on real device

**New files created:**
- `KtorBenchmarkServer.kt` - JVM-compatible Ktor/Netty WebSocket server for isolated benchmarking

**New tests added:**
- `ktor_100MB`, `ktor_10MB`, `ktor_LargeChunks` - Fixed-size Ktor benchmarks
- `ktor_Sustained30s`, `standalone_Sustained30s` - Sustained throughput over 30 seconds
- `ktor_vs_JavaWebSocket` - Side-by-side comparison test
- `external_100MB`, `external_10MB`, `external_Sustained30s` - External daemon tests (requires env vars)

**Run benchmarks:**
```bash
# Quick comparison (recommended first run)
./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest.ktor_vs_JavaWebSocket"

# Full benchmark suite
./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest*"

# External daemon (requires DAEMON_TOKEN env var)
DAEMON_TOKEN=<token> ./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest.external*"
```

**Baseline Results (JVM, macOS M3):**

| Server Type | Throughput | Frames/s | Frame Size |
|-------------|------------|----------|------------|
| java-websocket (TestDaemonServer) | **1714 MB/s** | ~10,000 | 55 KB avg |
| Ktor/Netty (KtorBenchmarkServer) | **201 MB/s** | ~3,800 | 55 KB avg |

**Key Finding:** Ktor is **8.4x slower** than java-websocket on JVM with identical protocol handling.
This confirms the hypothesis that Ktor WebSocket is the bottleneck.

**Detailed Ktor metrics (100 MB transfer):**
- Frame timing: 258 µs avg, 870 µs P99
- Frame distribution: 66% at max size (64KB+), 19% at 32-64KB

### Phase 2: Create Abstraction Layer ✅ COMPLETE

**Goal:** Decouple handlers from Ktor-specific types.

**New files created:**
- `websocket/WebSocketSession.kt` - Transport-agnostic WebSocket session interface
- `websocket/KtorWebSocketSession.kt` - Ktor implementation wrapping `DefaultWebSocketServerSession`

**Interface:**
```kotlin
interface WebSocketSession {
    suspend fun receive(): ByteArray?  // null on close
    suspend fun send(data: ByteArray)
    suspend fun close(code: Int = 1000, reason: String = "")
    val isOpen: Boolean
}
```

**Tasks:**
1. [x] Create `WebSocketSession` interface
2. [x] Create `KtorWebSocketSession` implementation wrapping `DefaultWebSocketServerSession`
3. [x] Refactor `IoWebSocketHandler` to accept `WebSocketSession` instead of Ktor type
4. [x] Refactor `ControlWebSocketHandler` similarly
5. [x] Verify all existing tests still pass

**Key changes:**
- Both handlers now take `WebSocketSession` in constructor instead of `DefaultWebSocketServerSession`
- The main receive loop uses `while (true) { session.receive() ?: break }` instead of `for (frame in incoming)`
- The sender coroutine uses `session.send(data)` instead of `wsSession.send(Frame.Binary(...))`
- Close calls use `session.close(code, reason)` instead of Ktor's `CloseReason`
- `CompanionHttpServer` wraps Ktor sessions with `KtorWebSocketSession(this)` when creating handlers
- Added `closeSession()` method to `ControlWebSocketHandler` for external close (replaces direct Ktor access)

### Phase 3: Implement Raw Netty WebSocket ✅ COMPLETE

**Goal:** Direct Netty WebSocket without Ktor abstraction layer.

**New files created:**
- `websocket/NettyWebSocketSession.kt` - Raw Netty channel wrapper implementing `WebSocketSession`
- `websocket/NettyWebSocketServer.kt` - Direct Netty WebSocket server with HTTP upgrade handling
- `benchmark/NettyBenchmarkServer.kt` - JVM-compatible benchmark server for testing

**Implementation highlights:**
- `NettyWebSocketSession` wraps a Netty `ChannelHandlerContext` with a Kotlin channel for async receive
- `NettyWebSocketServer` uses Netty's `ServerBootstrap` with custom pipeline:
  - `HttpServerCodec` → `HttpObjectAggregator` → `WebSocketUpgradeHandler` → `WebSocketFrameHandler`
- Supports multiple endpoints via `addEndpoint(path, handler)`

**Tasks:**
1. [x] Add direct Netty dependency (already transitive via Ktor)
2. [x] Implement `NettyWebSocketServer` with HTTP upgrade and routing
3. [x] Implement `NettyWebSocketSession` wrapping Netty channel
4. [x] Handle HTTP upgrade and WebSocket handshake
5. [x] Route `/io` and `/control` paths
6. [x] Benchmark against Ktor baseline

**Run benchmarks:**
```bash
# Quick comparison of all three implementations
./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest.all_servers_comparison"

# Individual Netty tests
./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest.netty*"
```

**Phase 3 Results (JVM, macOS M3):**

| Server Type | Throughput | Frames/s | vs Ktor |
|-------------|------------|----------|---------|
| java-websocket (TestDaemonServer) | **1753 MB/s** | ~10,000 | 8.5x faster |
| Raw Netty (NettyBenchmarkServer) | **1467 MB/s** | ~10,000 | **7.1x faster** |
| Ktor/Netty (KtorBenchmarkServer) | **207 MB/s** | ~2,100 | baseline |

**Key Finding:** Raw Netty is **7.1x faster** than Ktor on JVM with identical protocol handling.
This confirms that bypassing Ktor's WebSocket abstraction layer dramatically improves throughput.

**Analysis:**
- Ktor overhead vs Raw Netty: **85.9%** of time is spent in Ktor abstractions
- Ktor overhead vs java-ws: **88.2%**
- java-websocket vs Raw Netty: only 1.2x faster (minimal difference)

**Conclusion:** Either Raw Netty or java-websocket would be suitable replacements for Ktor.

## Decision: Hybrid Architecture (Ktor HTTP + java-websocket IO)

Based on Phase 3 benchmarks, **java-websocket** is selected for the `/io` endpoint:

| Factor | java-websocket | Raw Netty |
|--------|---------------|-----------|
| **Throughput** | 1753 MB/s | 1467 MB/s |
| **Simplicity** | ✅ Simple API | ❌ Complex pipeline |
| **Already in codebase** | ✅ TestDaemonServer | Partial |
| **Lines of code** | ~150 | ~300 |

**Rationale:**
1. **Slightly faster** (1.2x over Raw Netty, 8.5x over Ktor)
2. **Much simpler** - no Netty pipeline management, no handler lifecycle complexity
3. **Already working** - `TestDaemonServer` is essentially the production implementation
4. **Fewer bugs** - simpler code = less to go wrong

**Architecture:** Hybrid with separate ports:
- **Port 7800 (Ktor):** HTTP routes + `/control` WebSocket (low-volume control plane)
- **Port 7801 (java-websocket):** `/io` WebSocket only (high-throughput data plane)

This avoids rewriting all HTTP routes while getting the performance benefit where it matters.

### Phase 4: Java-WebSocket Validation ✅ COMPLETE

**Goal:** Confirm java-websocket performance matches TestDaemonServer benchmarks.

**Status:** Already validated in Phase 3 benchmarks - `TestDaemonServer` uses java-websocket and achieves **1753 MB/s**.

**Tasks:**
1. [x] Benchmark java-websocket against Ktor baseline (done in Phase 3)
2. [x] Confirm simpler than Raw Netty (done - ~150 vs ~300 lines)
3. [x] Verify Android compatibility (already used in test infrastructure)

### Phase 5: Android Companion - java-websocket IO Server ✅ COMPLETE

**Goal:** Add java-websocket server for `/io` endpoint on separate port.

**New files created:**
- `websocket/JavaWebSocketSession.kt` - java-websocket wrapper implementing `WebSocketSession`
- `websocket/JavaWebSocketServer.kt` - Production `/io` server based on `TestDaemonServer`
- `websocket/JavaWebSocketSessionTest.kt` - Unit tests for the session wrapper

**Tasks:**
1. [x] Create `JavaWebSocketSession` implementing `WebSocketSession` interface
2. [x] Create `JavaWebSocketServer` handling `/io` path with auth handshake
3. [x] Integrate into `CompanionHttpServer` - start java-websocket on port 7801
4. [x] Add `ioPort` field to `/status` response
5. [x] Remove `/io` route from Ktor (keep `/control` on Ktor)
6. [x] Unit tests for `JavaWebSocketSession`

**Files modified:**
- `CompanionHttpServer.kt` - start/stop java-websocket server, add `ioPort` to status
- `StatusResponse` data class - add `ioPort: Int?` field
- `build.gradle.kts` - added java-websocket dependency

**Architecture:**
- Ktor server on port 7800: HTTP routes + `/control` WebSocket
- Java-websocket server on port 7801: `/io` WebSocket only (high-throughput)
- `ioPort` is returned in `/status` response for clients to use
- If java-websocket server fails to start, `ioPort` will be null in status

### Phase 6: Extension Updates ✅ COMPLETE

**Goal:** Extension reads `ioPort` from status and passes to engine.

**Files modified:**

1. **`extension/src/lib/native-connection.ts`**
   - Added `ioPort?: number` to `DaemonInfo` interface

2. **`extension/src/lib/daemon-bridge.ts`**
   - `fetchStatus()` return type - includes `ioPort`
   - `completeConnection()` - reads `ioPort` from status, stores in daemon info
   - All call sites updated to pass `ioPort`

3. **`packages/client/src/types.ts`**
   - Added `ioPort?: number` to `DaemonInfo` interface (for engine-manager use)

**Tasks:**
1. [x] Add `ioPort?: number` to `DaemonInfo` interface in `native-connection.ts`
2. [x] Update `fetchStatus()` return type in `daemon-bridge.ts`
3. [x] Store `ioPort` in daemon info via `completeConnection()`
4. [x] Find where engine `DaemonConnection` is created, pass `ioPort`
5. [x] TypeScript type-check passes: `cd extension && pnpm typecheck`

### Phase 7: Engine/Client Updates ✅ COMPLETE

**Goal:** Engine's `DaemonConnection` connects `/io` to separate port.

**Files modified:**

1. **`packages/engine/src/adapters/daemon/daemon-connection.ts`**
   - Constructor: added optional `ioPort` parameter (defaults to `port` for backward compat)
   - `connectWebSocket()`: uses `this.ioPort ?? this.port` for WebSocket URL

2. **`packages/client/src/engine-manager/chrome-extension-engine-manager.ts`**
   - Both ChromeOS and desktop paths now pass `daemonInfo.ioPort` to `DaemonConnection`

3. **`packages/client/src/engine-manager/android-standalone-engine-manager.ts`**
   - Not modified - standalone mode doesn't need separate ioPort (in-process)

**Tasks:**
1. [x] Add `ioPort?: number` parameter to `DaemonConnection` constructor
2. [x] In `connectWebSocket()`, use `this.ioPort ?? this.port` for WebSocket URL
3. [x] Update `chrome-extension-engine-manager.ts` to pass `daemonInfo.ioPort`
4. [x] Update `android-standalone-engine-manager.ts` if needed (not needed)
5. [x] TypeScript type-check passes: `cd packages/engine && pnpm typecheck`
6. [x] TypeScript type-check passes: `cd packages/client && pnpm typecheck`

### Phase 8: Integration Testing & Validation ✅ COMPLETE

**Goal:** Verify end-to-end throughput improvement.

**Tests to run:**
```bash
# Unit tests (JVM)
./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest*"
./gradlew :companion-server:testDebugUnitTest

# Instrumented tests (emulator)
./gradlew :app:connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=com.jstorrent.app.companion.WebSocketIOTest
./gradlew :app:connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=com.jstorrent.app.companion.TcpSocketTest

# Extension tests
cd extension && pnpm typecheck && pnpm test
cd packages/engine && pnpm typecheck && pnpm test
cd packages/client && pnpm typecheck
```

**Tasks:**
1. [x] All existing unit tests pass
2. [x] All existing instrumented tests pass (with ioPort fixes)
3. [x] Extension builds and type-checks
4. [x] Engine builds and type-checks (1138 tests passed)
5. [ ] Manual test on ChromeOS with real torrent
6. [ ] Measure throughput: target **40+ MB/s** (vs 12-14 MB/s baseline)

**Files modified during Phase 8:**
- `IoDaemonService.kt` - Added `ioPort` property to expose java-websocket server port
- `WebSocketIOTest.kt` - Updated to connect to `ioPort` instead of `port` for /io endpoint
- `TcpSocketTest.kt` - Updated to connect to `ioPort` instead of `port` for /io endpoint
- `CompanionTestBase.kt` - Wait for `ioPort > 0` before running tests, increased timeout

**Phase 8 Benchmark Results (JVM, macOS M3):**
```
java-websocket: 1597.90 MB/s (1645 frames)
Ktor/Netty:     202.52 MB/s (2008 frames)

=== Analysis ===
java-websocket is 7.9x faster than Ktor
Ktor overhead: 87.3%
```

### Phase 9: Cleanup (Optional)

**Goal:** Remove unused code after successful migration.

**Tasks:**
1. [ ] Remove `/io` WebSocket route from Ktor config
2. [ ] Remove `KtorWebSocketSession` if no longer used
3. [ ] Remove Netty WebSocket files if not needed (from Phase 3 experiments)
4. [ ] Update documentation

## Test Plan for Migration

### Phase 5 Tests (Android Companion)

```bash
# Existing tests must pass
./gradlew :companion-server:testDebugUnitTest
./gradlew connectedDebugAndroidTest --tests "*WebSocket*"

# New tests to add
# - JavaWebSocketSessionTest.kt - unit test for session wrapper
# - JavaWebSocketServerTest.kt - unit test for server startup/auth
# - DualPortIntegrationTest.kt - verify /io on 7801, /control on 7800
```

### Phase 6-7 Tests (Extension + Engine)

```bash
# Extension
cd extension && pnpm typecheck && pnpm test

# Engine
cd packages/engine && pnpm typecheck && pnpm test

# Integration (if available)
cd packages/engine && pnpm test:integration
```

### Phase 8 Tests (End-to-End)

```bash
# Full instrumented test suite
./gradlew connectedDebugAndroidTest

# Throughput benchmark
./gradlew :app:testDebugUnitTest --tests "*ThroughputBenchmarkTest.ktor_vs_JavaWebSocket"

# Manual ChromeOS test
# 1. Deploy to Chromebook: ./scripts/deploy-chromebook.sh
# 2. Start a real torrent download
# 3. Monitor throughput in logs (target: 40+ MB/s)
```

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Port conflict on 7801 | Use same fallback formula as 7800 (7801, 7806, 7815...) |
| Extension/engine mismatch | Phase 6+7 can be done together to avoid partial state |
| java-websocket threading issues | Use same patterns as TestDaemonServer (proven working) |
| Performance still poor | Profile with Android Studio, check for other bottlenecks |

## Success Criteria

- [ ] Sustained throughput of **40+ MB/s** on ChromeOS (vs current 12-14 MB/s) - **Pending ChromeOS testing**
- [x] No regression in existing test suite - **All tests pass (unit + instrumented)**
- [ ] No increase in CPU usage (currently ~5% for TCP read path) - **Pending ChromeOS testing**
- [x] Extension connects to correct ports automatically - **ioPort plumbed through DaemonConnection**

## Timeline Estimate

| Phase | Effort | Status |
|-------|--------|--------|
| Phase 1: Baseline benchmarks | 1 day | ✅ Complete |
| Phase 2: Abstraction layer | 1-2 days | ✅ Complete |
| Phase 3: Raw Netty impl | 2-3 days | ✅ Complete |
| Phase 4: Java-WebSocket validation | 0.5 day | ✅ Complete |
| Phase 5: Android java-websocket server | 1 day | ✅ Complete |
| Phase 6: Extension updates | 0.5 day | ✅ Complete |
| Phase 7: Engine updates | 0.5 day | ✅ Complete |
| Phase 8: Integration testing | 0.5 day | ✅ Complete |
| Phase 9: Cleanup | 0.5 day | Optional |
| **Total** | **7-9 days** | **~8 days done** |

## References

- [Ktor WebSocket 10x slower than OkHttp issue](https://github.com/ktorio/ktor/issues/982)
- [Java-WebSocket library](https://github.com/TooTallNate/Java-WebSocket)
- [Netty WebSocket documentation](https://netty.io/wiki/related-projects.html)
- [C1000K WebSocket benchmarks](https://github.com/smallnest/C1000K-Servers)
