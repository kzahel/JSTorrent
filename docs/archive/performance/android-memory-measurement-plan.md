# Android Memory Measurement Plan

**Status:** Authoritative plan
**Date:** 2026-03-06
**Supersedes:** [android-memory-optimization.md](android-memory-optimization.md)

## Purpose

Establish a measurement-first plan for Android memory work.

At the moment we know one hard fact: large Android downloads can be killed by
the Low Memory Killer (LMK), especially when the app is backgrounded or the user
multitasks. Nearly everything else is still hypothesis.

This document defines the work we should do now:

1. Add visibility into process, runtime, and engine memory state.
2. Capture reproducible memory traces for real download scenarios.
3. Use those traces to decide whether the dominant problem is:
   - overly large active piece working set
   - endgame spikes
   - buffer pool retention
   - peer/swarm growth
   - JNI transient allocation churn
   - JVM heap pressure
   - missing reaction to `onTrimMemory()`
   - some combination of the above

This phase does not attempt to fix memory pressure yet.

## What We Actually Know

- Android can kill the process during large downloads.
- The current native `ActivePieceManager` defaults are still high:
  - `maxActivePieces = 128`
  - `maxBufferedBytes = 64 MiB`
- The codebase already contains a comment that Android standalone can go OOM
  near the end of a download if `maxActivePieces` is too high.
- There is currently no app-level `onTrimMemory()` handling in
  `JSTorrentApplication`.
- The QuickJS runtime exposes `JS_ComputeMemoryUsage()`,
  `JS_RunGC()`, and `JS_SetMemoryLimit()`, but we are not yet using those to
  instrument live behavior.
- The running `jstorrent-dev` emulator is configured with `2048 MiB` guest RAM.

## What We Do Not Yet Know

- Whether the main driver is JS heap growth, native heap growth, JVM growth, or
  total RSS from all of them combined.
- Whether memory climbs gradually, spikes during endgame, or spikes when the app
  is backgrounded.
- Whether Android is sending `onTrimMemory()` warnings before the kill.
- Whether the app can survive current workloads with lower steady-state limits
  and no other architectural changes.
- Whether background multitasking is required to reproduce the problem.
- Whether emulator-only behavior differs materially from a real low-RAM device.

## Scope

This phase is limited to measurement and observability.

### In scope

- Add memory instrumentation on Android, Kotlin, JNI, and engine sides.
- Add a single structured memory snapshot view that can be logged and queried.
- Record trim-memory callbacks and process state transitions.
- Run a small number of reproducible download scenarios and capture traces.
- Produce a decision-oriented summary after the data is collected.

### Out of scope

- Dynamic memory reduction logic
- Piece eviction
- Peer-cap tuning
- Endgame policy changes
- Lowering limits by guesswork
- `JS_SetMemoryLimit()` safety caps
- `largeHeap=true`

Those may be valid later, but not before the data exists.

## Decision Principles

### 1. Prefer raw numbers over inferred explanations

If a claim cannot be tied to a measured counter or trace, treat it as a
hypothesis.

### 2. Measure before changing limits

Changing limits before baseline data makes the baseline disappear and turns the
exercise into guess-and-check.

### 3. Separate signal collection from mitigation

Instrumentation should be useful even if every mitigation idea changes later.

### 4. Optimize for repeatability

A slightly artificial but repeatable scenario is more valuable than an
uncontrolled anecdotal failure.

## Deliverable

The output of this phase is not “memory is fixed.”

The output is:

- a reliable memory snapshot API
- periodic memory logging while downloads run
- trim-memory event logging
- baseline traces for a defined test matrix
- a follow-up design note grounded in actual measurements

## Measurement Questions

The instrumentation must let us answer the following questions directly:

1. What is total process memory doing over time during a large download?
2. How much of that memory is:
   - QuickJS heap
   - native heap outside QuickJS stats
   - JVM heap
   - active piece buffers
   - pooled piece buffers
3. Does memory growth correlate with:
   - torrent progress
   - endgame entry
   - number of active pieces
   - number of connected peers
   - known swarm size
   - DHT activity
4. Do `onTrimMemory()` callbacks happen before the app is killed?
5. If trim callbacks happen, how much time do we have between first warning and
   kill?
6. Is the app already above a reasonable steady-state budget before any trim
   event?
7. Does backgrounding the app materially change memory or kill probability?

## Proposed Instrumentation

### 1. Unified memory snapshot

Define one logical snapshot returned by the app on demand and emitted in logs on
an interval.

### Snapshot fields

#### Android / process

- timestamp
- app foreground/background state
- last `onTrimMemory()` level and time
- process `Pss`, `Rss`, and private dirty if available from Android APIs
- native heap allocated size from `Debug.getNativeHeapAllocatedSize()`
- JVM heap:
  - used
  - free
  - max
- optional low-memory state from `ActivityManager.MemoryInfo`

#### QuickJS

- total runtime memory from `JS_ComputeMemoryUsage()`
- malloc size
- object count
- array count
- typed array count
- atom size / count
- string size / count
- shape / property stats if available

We do not need every QuickJS field in human logs, but the structured snapshot
should preserve them.

#### Engine

- torrent count
- active downloading torrent count
- per-torrent:
  - progress
  - download rate
  - active piece counts by state:
    - partial
    - fully requested
    - fully responded
  - total buffered bytes
  - peak buffered bytes
  - piece length
  - buffer pool bytes
  - buffer pool size
  - buffer pool hit/miss counts or hit rate
  - connected peers
  - connecting peers
  - known peers
  - DHT node count / peer-store size where applicable

### Requirements

- One snapshot call should be cheap enough to run every 30 seconds during active
  downloads.
- It must not allocate large temporary strings on the JS side.
- The same snapshot shape should be used by:
  - debug broadcast command
  - periodic logging
  - later automated tests

### 2. `onTrimMemory()` visibility only

Implement `onTrimMemory()` now, but only for measurement.

### What it should do in this phase

- record last trim level and timestamp
- log the callback with a stable tag, for example `JSTorrent-Mem`
- optionally increment counters by trim level

### What it should not do in this phase

- no limit reduction
- no pausing torrents
- no explicit JS GC triggered from trim callbacks
- no piece eviction

The callback is instrumentation, not mitigation.

### 3. Debug broadcast command

Extend the existing debug receiver with a `memory` command:

```bash
adb shell am broadcast -a com.jstorrent.DEBUG --es cmd memory
```

### Output goals

- readable in `logcat`
- includes one concise summary line
- followed by a structured breakdown
- safe to run during an active download

### Example summary

```text
[MEM] rss:212M pss:205M native:146M jvm:28/192M js:91M pieces:44 buf:31M pool:8M peers:18 known:240 trim:RUNNING_LOW@12s
```

### 4. Periodic logging

While at least one torrent is actively downloading, emit a periodic memory
sample every 30 seconds.

### Requirements

- Log with a dedicated tag such as `JSTorrent-Mem`
- Use one stable summary-line format to make traces grep-friendly
- Include torrent progress and endgame state when available
- Avoid per-peer or per-piece verbose detail in the periodic line

### Optional enhancement

In addition to logcat, write structured JSONL samples to app-private storage so
we can analyze them after a kill or after long runs. This is useful but not
required for the first pass.

If JSONL is added, each line should be a single snapshot object with a schema
version.

### 5. Important lifecycle markers

Memory samples are only useful if they can be aligned with runtime events.

Add log markers for:

- torrent start
- torrent pause/resume
- app moved to background
- app returned to foreground
- endgame entered
- endgame exited
- download completed
- major peer disconnect wave if already observable

These should be simple structured log entries using the same memory tag or a
clearly related tag.

## Test Matrix

The first round should be intentionally small. The goal is to answer the basic
questions, not exhaust every device permutation.

### Scenario A: Baseline foreground download

- Device: current `jstorrent-dev` emulator at `2048 MiB`
- Workload: one large torrent that has previously shown the issue or is likely
  to stress active pieces
- App stays in foreground

### Capture

- periodic memory logs
- manual `memory` snapshots at:
  - start
  - 25%
  - 50%
  - 75%
  - 90%
  - endgame
  - completion or failure

### Questions answered

- Does memory climb even without multitasking?
- Is memory mostly stable outside endgame?

### Scenario B: Background multitasking

Same download, but after a stable transfer starts:

- send app to background
- open several Android apps inside the emulator
- return to JSTorrent periodically

### Capture

- periodic memory logs
- trim callback logs
- manual `memory` snapshots before backgrounding and after returning

### Questions answered

- Does backgrounding trigger trim callbacks?
- Does kill risk depend on multitasking?

### Scenario C: Forced trim callback validation

Use Android shell to inject trim events after instrumentation is in place:

```bash
adb shell am send-trim-memory com.jstorrent.app RUNNING_LOW
adb shell am send-trim-memory com.jstorrent.app RUNNING_CRITICAL
```

### Purpose

- verify the callback path works
- verify logs include the level and timestamp
- verify debug snapshots show the last trim event

This scenario validates measurement plumbing only. It does not prove real LMK
behavior.

### Scenario D: Endgame observation

Capture one run with special attention to the last 5-10% of a large download.

### Questions answered

- Does memory spike at endgame?
- Does the number of fully requested or fully responded pieces jump?
- Does the piece buffer pool retain memory after completion?

### Scenario E: Lower-RAM emulator

Only after Scenarios A-D are complete, repeat the baseline on a smaller-RAM AVD.

### Recommended order

1. `1536 MiB`
2. `1024 MiB` if needed

### Why not start here

Starting with a constrained AVD before baseline risks optimizing for an
artificial failure mode without understanding normal behavior on the current
configuration.

## Metrics To Compare Across Runs

- peak RSS / PSS
- peak QuickJS memory
- peak native heap
- peak JVM heap used
- peak active pieces
- peak buffered bytes
- peak pool bytes
- peak connected peers
- known peers at peak memory
- whether trim callbacks occurred
- whether kill occurred
- approximate progress at failure
- whether failure clustered around endgame

## Success Criteria For This Phase

This phase is complete when all of the following are true:

1. We can request a memory snapshot on demand from `adb`.
2. We get periodic memory samples during active downloads.
3. We log every `onTrimMemory()` callback with level and time.
4. We have at least one successful baseline trace and one stressed trace.
5. We can say, with evidence, which subsystem dominates memory at the point of
   highest risk.

## Explicit Non-Goals

The following are intentionally deferred until after the measurement phase:

- picking a new `maxActivePieces`
- picking a new `maxBufferedBytes`
- enabling periodic `JS_RunGC()`
- adding `JS_SetMemoryLimit()`
- adding buffer-pool clearing heuristics
- adding piece eviction
- capping swarm peers
- pausing torrents on memory pressure

Any of those could be correct. None should be merged on guesswork alone.

## Recommended Follow-Up After Measurement

Once the traces are collected, write a second document that answers:

- What is the primary memory driver?
- What are the top one or two interventions with the highest expected impact?
- What evidence supports those interventions?
- What should be measured again after the first mitigation lands?

That follow-up document should replace speculation with a ranked change list.

## Implementation Checklist

- [ ] Add app-level `onTrimMemory()` logging and last-seen state
- [ ] Expose QuickJS memory stats through JNI
- [ ] Add engine `getMemoryStats()` aggregation
- [ ] Add `adb` debug receiver `memory` command
- [ ] Add periodic memory logging during active downloads
- [ ] Add lifecycle markers for background/foreground/endgame/completion
- [ ] Run Scenario A and save notes
- [ ] Run Scenario B and save notes
- [ ] Run Scenario C and verify trim logging
- [ ] Run Scenario D and save notes
- [ ] Decide whether Scenario E is needed immediately
