# 001: Repository Housekeeping

Status: active.

Refresh JSTorrent's aging dependencies and development infrastructure without
combining the work with unrelated product migrations. This tactical is the
durable plan and execution record for the campaign.

Last reconciled: 2026-07-23.

## Motivation

The repository had recently received only the minimum Android target-SDK
update. A broader audit found end-of-life runtimes, known dependency
advisories, inconsistent CI setup, stale scripts and generated artifacts, and
release installers without integrity verification.

The desired outcome is a supportable baseline with green local and hosted
gates. Changes should be split into logical commits so regressions remain easy
to attribute.

## Constraints

- Keep behavior changes out of dependency and infrastructure commits.
- Preserve unrelated work already on `main`.
- Commit each logical checkpoint independently.
- Use Node 24 through NVM locally and the same major version in CI.
- Continue using uv for Python projects; a uv-managed local `.venv` is expected
  and remains ignored.
- Keep the Android target/compile SDK at API 36; API 37 local-network permission
  work remains deferred in
  [`../topics/android-api-37-local-network-permission.md`](../topics/android-api-37-local-network-permission.md).
- Push only after local validation, then monitor hosted CI and fix failures
  until the required gates are green.

## Explicitly Out Of Scope

- Dependabot or Renovate configuration
- React 19 migration
- Vite 8 migration
- TypeScript 7 migration
- QuickJS-NG submodule upgrade
- Android API 37 local-network permission implementation
- deleting ignored build products or caches solely to reclaim local disk space

## Audit Baseline

The 2026-07-23 read-only audit found:

- Node 20 was pinned by the repository even though it had reached end of life;
  the development shell exposed the also-end-of-life Node 25.
- pnpm was pinned to the 9.x line.
- `pnpm audit --prod` reported 57 advisory paths: 27 high, 23 moderate,
  7 low, and no critical findings. Important direct or near-direct paths
  included Solid/seroval, `bn.js`, `happy-dom`, Rollup, Vite, and Astro.
- Android's network-facing companion server used Netty `4.1.100.Final`, a
  version affected by multiple published network-protocol advisories.
- `cargo audit` reported 16 vulnerability records across the desktop lockfile,
  including TLS, updater, archive, time, and XML parsing paths.
- Both Python projects pinned Python 3.10. Their locks contained advisories in
  packages including requests, urllib3, idna, and aiohttp.
- Several workflows used mutable package installs, broad write permissions,
  older action majors, or unpinned global Python installation.
- Public installer scripts downloaded release assets without checksum
  verification.
- Tracked backup, log, translation-output, and obsolete helper files remained
  in the tree.

Audit counts are dependency/advisory paths, not claims that every finding is
independently exploitable in a shipped application. Validation and dependency
path review remain part of each update slice.

## Work Plan

### 1. JavaScript Security Baseline

- [ ] Apply low-risk direct security updates, including current compatible
  Solid, `bn.js`, and `happy-dom` releases.
- [ ] Consolidate packages still on Vitest 1 onto the current Vitest line and
  remove the corresponding older Vite dependency path.
- [ ] Remove unused `ts-node` if repository-wide usage remains absent.
- [ ] Replace the unmaintained `npm-run-all` package with a maintained
  equivalent.
- [ ] Align workspace TypeScript ranges before deduplicating the lockfile.
- [ ] Update the website's affected Astro/Vite/Rollup/sharp dependency chain.
  Keep the Astro major migration isolated because it may require source or
  configuration changes.
- [ ] Re-run the production audit and document remaining accepted or deferred
  findings.

Validation:

```bash
pnpm install --frozen-lockfile
pnpm lint
pnpm format
pnpm docs:check
pnpm typecheck
pnpm test
pnpm build
pnpm audit --prod
```

### 2. Android Network Dependency Update

- [ ] Centralize the hard-coded Netty version in the Gradle version catalog.
- [ ] Update Netty from `4.1.100.Final` to a non-vulnerable current 4.1.x
  release.
- [ ] Compile Android and run unit tests, companion-server tests, and applicable
  end-to-end coverage.

Validation:

```bash
cd android
./gradlew :app:compileDebugKotlin
./gradlew testDebugUnitTest
```

### 3. Rust Security Baseline

- [ ] Apply compatible lockfile updates for vulnerable transitive packages.
- [ ] Update Tauri and plugins where compatible releases resolve updater or
  runtime advisories.
- [ ] Upgrade the desktop host's old direct `reqwest 0.11` dependency and adapt
  code only where the new API requires it.
- [ ] Re-run `cargo audit`, classify any residual platform/build-only paths,
  and record deliberate deferrals.

Validation:

```bash
cd desktop
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo audit
```

### 4. Python And uv Refresh

- [ ] Move project Python pins from 3.10 to 3.12, subject to libtorrent
  compatibility.
- [ ] Fix the desktop Python project name and placeholder metadata mismatch.
- [ ] Refresh and audit both uv lockfiles.
- [ ] Give the iOS App Store Connect script a small locked uv project and
  replace unpinned `pip install --break-system-packages` workflow steps with
  `uv run`.
- [ ] Add immutable lock checks to relevant CI jobs.

Validation:

```bash
cd desktop
uv lock --check
uv run python --version

cd packages/engine/integration/python
uv lock --check
uv run python --version
```

Run Python dependency auditing against both resolved projects and run the
integration Python tests used by the root workspace.

### 5. Supported Toolchains And CI Reproducibility

- [ ] Install or update NVM on this development machine from its official
  source.
- [ ] Pin Node 24 in `.nvmrc`, package engines, and workflows.
- [ ] Reassess the Node minimum advertised by published packages and test every
  advertised supported major.
- [ ] Upgrade pnpm after Node 24 is active and validate the resulting lockfile.
- [ ] Pin a Rust toolchain for release reproducibility.
- [ ] Update maintained GitHub Actions to current supported majors.
- [ ] Use `pnpm install --frozen-lockfile` consistently in CI.
- [ ] Replace direct uv installer piping with the maintained setup action.
- [ ] Default workflow permissions to `contents: read` and grant write only to
  jobs that publish, tag, or intentionally commit.
- [ ] Add dependency-audit gates for pnpm, Cargo, and Python without adding
  dependency-update automation.

Validation includes every root static/test gate plus workflow syntax review and
the hosted CI run after push.

### 6. Stale Tooling And Artifact Cleanup

- [ ] Remove tracked `.bak`, `.raw`, `.err`, and debug log artifacts that have
  no source role; adjust narrow ignore patterns where useful.
- [ ] Remove or replace desktop Python verifier scripts that invoke the removed
  `jstorrent-link-handler` binary.
- [ ] Remove obsolete `system-bridge` installer/version references and stale
  monorepo guidance.
- [ ] Review apparently orphaned scripts individually and remove only those
  whose callers and purpose are conclusively obsolete.

Validation:

```bash
pnpm docs:check
pnpm format
git grep -n 'system-bridge\\|jstorrent-link-handler'
git diff --check
```

Remaining matches must be current compatibility context rather than executable
stale paths.

### 7. Release Asset Integrity

- [ ] Generate SHA-256 checksums for desktop release artifacts.
- [ ] Attach the checksum manifest to the GitHub release.
- [ ] Make supported public installer scripts download and verify the manifest
  before installing executable assets.
- [ ] Fail closed on missing or mismatched checksums.
- [ ] Add shell-level tests or fixtures for success, mismatch, and missing
  checksum behavior.

### 8. Closeout

- [ ] Run the complete local gate set under the pinned Node version.
- [ ] Record final audit results and any narrowly justified residual findings
  in this tactical.
- [ ] Mark each landed work item and this tactical completed.
- [ ] Push the complete commit series.
- [ ] Monitor every required hosted CI job and repair failures until green.

## Acceptance Criteria

- Supported local and CI JavaScript work uses Node 24.
- Lockfiles are current, reproducible, and pass their ecosystem checks.
- No known direct high-severity dependency finding remains without an explicit,
  evidence-based deferral in this record.
- Android companion-server network dependencies no longer use the vulnerable
  Netty baseline.
- The Rust and Python audit results are either clean or contain only documented
  residual transitive/platform findings with a follow-up owner.
- Required local lint, formatting, documentation, type, unit, build, Android,
  Rust, Python, and installer-integrity checks pass.
- Hosted CI is green after the commit series is pushed.

## Execution Record

This section will be updated at each checkpoint with commits, important
decisions, validation results, audit deltas, and explicit deferrals.
