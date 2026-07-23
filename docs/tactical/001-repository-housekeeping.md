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
- standalone Vite 8 migration outside Astro's website-owned build toolchain
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

- [x] Apply low-risk direct security updates, including current compatible
  Solid, `bn.js`, and `happy-dom` releases.
- [x] Consolidate packages still on Vitest 1 onto the current Vitest line and
  remove the corresponding older Vite dependency path.
- [x] Remove unused `ts-node` if repository-wide usage remains absent.
- [x] Replace the unmaintained `npm-run-all` package with a maintained
  equivalent.
- [x] Align workspace TypeScript ranges before deduplicating the lockfile.
- [x] Update the website's affected Astro/Vite/Rollup/sharp dependency chain.
  Keep the Astro major migration isolated because it may require source or
  configuration changes.
- [x] Re-run the production audit and document remaining accepted or deferred
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

- [x] Centralize the hard-coded Netty version in the Gradle version catalog.
- [x] Update Netty from `4.1.100.Final` to a non-vulnerable current 4.1.x
  release.
- [x] Compile Android and run unit tests, companion-server tests, and applicable
  end-to-end coverage.

Validation:

```bash
cd android
./gradlew :app:compileDebugKotlin
./gradlew testDebugUnitTest
```

### 3. Rust Security Baseline

- [x] Apply compatible lockfile updates for vulnerable transitive packages.
- [x] Update Tauri and plugins where compatible releases resolve updater or
  runtime advisories.
- [x] Upgrade the desktop host's old direct `reqwest 0.11` dependency and adapt
  code only where the new API requires it.
- [x] Re-run `cargo audit`, classify any residual platform/build-only paths,
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

- [x] Move maintained Python projects from 3.10 to 3.12, subject to libtorrent
  compatibility.
- [x] Assess the desktop verifier environment; it was refreshed at its
  checkpoint, then removed with the conclusively obsolete verifier suite.
- [x] Refresh and audit the maintained uv lockfiles.
- [x] Give the iOS App Store Connect script a small locked uv project and
  replace unpinned `pip install --break-system-packages` workflow steps with
  `uv run`.
- [x] Add immutable lock checks to relevant CI jobs.

Validation:

```bash
cd packages/engine/integration/python
uv lock --check
uv run python --version

cd ../../../../ios
uv lock --check
uv run python scripts/appstore_connect.py --help
```

Run Python dependency auditing against both maintained projects and run the
integration Python tests used by the root workspace.

### 5. Supported Toolchains And CI Reproducibility

- [x] Install or update NVM on this development machine from its official
  source.
- [x] Pin Node 24 in `.nvmrc`, package engines, and workflows.
- [x] Reassess the Node minimum advertised by published packages and test every
  advertised supported major.
- [x] Upgrade pnpm after Node 24 is active and validate the resulting lockfile.
- [x] Pin a Rust toolchain for release reproducibility.
- [x] Update maintained GitHub Actions to current supported majors.
- [x] Use `pnpm install --frozen-lockfile` consistently in CI.
- [x] Replace direct uv installer piping with the maintained setup action.
- [x] Default workflow permissions to `contents: read` and grant write only to
  jobs that publish, tag, or intentionally commit.
- [x] Add dependency-audit gates for pnpm, Cargo, and Python without adding
  dependency-update automation.

Validation includes every root static/test gate plus workflow syntax review and
the hosted CI run after push.

### 6. Stale Tooling And Artifact Cleanup

- [x] Remove tracked `.bak`, `.raw`, `.err`, and debug log artifacts that have
  no source role; adjust narrow ignore patterns where useful.
- [x] Remove or replace desktop Python verifier scripts that invoke the removed
  `jstorrent-link-handler` binary.
- [x] Remove obsolete `system-bridge` installer/version references and stale
  monorepo guidance.
- [x] Review apparently orphaned scripts individually and remove only those
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

- [x] Generate SHA-256 checksums for desktop release artifacts.
- [x] Attach the checksum manifest to the GitHub release.
- [x] Make supported public installer scripts download and verify the manifest
  before installing executable assets.
- [x] Fail closed on missing or mismatched checksums.
- [x] Add shell-level tests or fixtures for success, mismatch, and missing
  checksum behavior.

### 8. Closeout

- [x] Run the complete local gate set under the pinned Node version.
- [x] Record final audit results and any narrowly justified residual findings
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

### Tactical Creation

Commit `29c9326e` created this tactical and its index before implementation
began. Documentation checks and focused Prettier validation passed.

### Node And pnpm Baseline

The development machine already had current NVM 0.40.3, so no installer script
was needed. NVM now has Node 24.18.0 installed and makes major 24 the default.
The repository pins Node 24 for local and CI use, and the published engine and
plugin SDK now advertise Node 24 as their sole supported major rather than
claiming untested support for end-of-life Node versions.

pnpm is pinned to 11.16.0. pnpm 11's dependency-build policy explicitly allows
the required esbuild and sharp install scripts; no other dependency lifecycle
scripts are approved.

Validation under Node 24.18.0 and pnpm 11.16.0:

- frozen-lockfile install passed
- documentation checks passed
- workspace typechecks passed
- workspace tests passed
- focused workflow and manifest formatting passed

### JavaScript Compatible Security Updates

The first dependency slice updated Solid to 1.9.14, `bn.js` to 5.2.5,
happy-dom to the mature patched 20.11.0 release, Vitest to 4.1.10, Vite 7 to
7.3.6 where already used, TypeScript 5 to 5.9.3, and related compatible
packages. It replaced `npm-run-all` with `npm-run-all2`, removed the unused
`ts-node`, and deduplicated the lockfile.

Vitest 4 required two mechanical test compatibility changes: test options now
precede callbacks, and constructor mocks use constructable functions. The
TypeScript alignment also replaced one `Array.prototype.at` call because the
engine deliberately targets an older JavaScript library surface.

pnpm 11 blocks exotic transitive sources by default. `playsvideo@0.4.7`
intentionally depends on JSTorrent's subtitle-integration fork of mediabunny,
so that sole edge is overridden to its previously locked immutable commit. The
global exotic-subdependency switch must remain disabled until playsvideo
publishes the integration through a registry dependency; the exact override
prevents the upstream branch name from moving silently.

Validation:

- static checks and all workspace builds passed
- 152 Vitest files passed across the engine and plugin SDK; the remaining
  client, UI, and extension suites also passed
- 2,109 tests passed and 1 live-network test remained intentionally skipped
- the production audit fell from 57 paths to 42 paths: 19 high, 18 moderate,
  and 5 low, all currently rooted in the Astro website dependency graph

### Astro 7 Website Migration

The website now uses Astro 7.1.3 and matching current official integrations.
Astro 7 necessarily owns a Vite 8 build internally; the standalone extension
and desktop Vite configurations remain on Vite 7 as planned. The migration also
moved Astro's checker to development dependencies and refreshed affected
Rollup, sharp, YAML, glob, and TOML parsing packages.

Astro 7 bundles prerendered component code under `dist/.prerender`, so the
changelog component can no longer derive the source repository from
`import.meta.dirname`. It now locates the monorepo root from the build working
directory. The content schema uses Astro's current Zod export, inline analytics
is explicit, and deprecated `navigator.platform` detection was removed.

Validation:

- `pnpm peers check` reported no peer dependency issues
- the complete workspace static checks and builds passed
- Astro check reported no errors, warnings, or hints
- all five static routes, the RSS feed, and the sitemap built
- the production dependency audit reported zero vulnerabilities
- the built site was served locally, the homepage returned valid content, and
  a full-page Chromium capture was visually inspected

### Android Netty Update

The companion server and its app benchmark fixtures now share Netty
4.1.136.Final through the Gradle version catalog instead of repeating
4.1.100.Final in build scripts.

Validation:

- companion-server debug unit tests passed
- app debug Kotlin compilation passed
- app debug unit tests passed
- the default emulator E2E gate passed against the real Python seeder; long
  100MB and 1GB completion tests are now explicit opt-in checks
- the build resolved the updated artifacts and completed successfully
- OSV returned no advisories for the selected codec-http, handler, or transport
  artifacts

The E2E validation exposed stale assumptions rather than a Netty regression:
the progress checks required 5MB within five seconds, and the default 100MB
seeder could never satisfy the 1GB completion test. The normal gate now waits
for an actual piece transfer, while the full-fixture tests use realistic
timeouts and explicit instrumentation flags.

The build still reports existing Gradle 9 migration warnings, chiefly the
quickjs module's deprecated `Project.exec` use. That infrastructure migration
is separate from the network dependency update and does not affect the current
Gradle 8.13 gate.

### Rust Security Update

The desktop workspace is pinned to Rust 1.97.0. Compatible lockfile updates
move Tauri to 2.11.5, refresh its updater and runtime chain, and remove the
vulnerable archive, XML, time, TLS, and concurrency versions from the baseline
audit. The native host now uses `reqwest` 0.13, eliminating its separate
`rustls` 0.21 and `rustls-webpki` 0.101 dependency path.

The Rust 1.97 clippy pass also exposed warnings that the former floating
toolchain had not made reproducible. Mechanical fixes were applied without
changing the protocol or daemon behavior.

Validation:

- `cargo fmt --all -- --check` passed
- `cargo clippy --workspace --all-targets -- -D warnings` passed
- `cargo test --workspace` passed, including Tauri, native-messaging, profile,
  and daemon tests
- `cargo audit` reported no vulnerabilities

The audit retains 18 warning-only records. Ten are the unmaintained GTK3
bindings used by Tauri's current Linux WebKit stack; seven are unmaintained
macro/Unicode transitive crates; and one is the known `glib` iterator
soundness warning. They have no compatible replacement in the current Tauri
2 Linux graph and remain visible rather than being added to an ignore list.

### Python And uv Update

The desktop tools and engine integration suite now require Python 3.12. Their
uv locks were rebuilt with current compatible dependencies, including
libtorrent 2.0.13, aiohttp 3.14.3, requests 2.34.2, and websockets 16.1.1. The
desktop project is now named `jstorrent-desktop-tools` and no longer carries
placeholder package metadata.

The App Store Connect helper now has its own `ios/pyproject.toml`, Python pin,
and uv lock. Both iOS release workflows use uv 0.9.18 with `--locked` rather
than mutating the hosted runner's system Python with an unpinned pip command.

Validation:

- all three `uv lock --check` commands passed
- all three environments resolved and ran on Python 3.12
- `pip-audit` reported no known vulnerabilities for any locked environment
- the iOS helper parsed its command line in the locked environment
- all 19 self-contained Python engine integration tests passed; the two
  external-seeder cases remained explicitly skipped by their existing runner

### CI Reproducibility And Audit Gates

Maintained first-party actions now use their current supported majors:
checkout 7, setup-node 7, setup-java 5, upload-artifact 7, cache 6, the Pages
actions 5/6, Gradle actions 6, Android setup 4, pnpm setup 6, uv setup 9, and
the GitHub release action 3. Pnpm is consistently 11.16.0 and every install is
frozen.

The disabled Android emulator jobs no longer contain a future copy-paste trap:
their direct uv installer and Python 3.11 setup were replaced with the same
locked Python 3.12 project used locally. Workflow defaults are read-only, with
write permission confined to release/deployment jobs.

The repository sanity gate now audits production JavaScript and the maintained
Python locks. The Linux Tauri job runs the pinned Rust formatter and clippy
gate and installs cargo-audit 0.22.2 for the Rust dependency gate.

### Stale Tooling And Artifact Cleanup

The tracked translation intermediates, debug log, and editor backup files had
no source role and are removed. Narrow ignore rules now prevent debug logs and
editor backups from being recommitted while preserving deliberate text
fixtures.

The desktop `verify_*.py` scripts had no callers or CI entry point. Several
invoked the removed `jstorrent-link-handler`; others duplicated maintained
Rust and protocol-conformance coverage, assumed a particular working
directory, or mutated developer-local state. The suite, its aggregator,
placeholder `main.py`, and the now-empty desktop Python environment are
removed. Python lock and audit CI now covers the two maintained environments:
the engine integration suite and iOS release tooling.

The public `install-bridge.sh` fetched superseded `system-bridge-v*` assets and
the version checker reported retired component tags, so both are removed.
Active desktop comments and diagnostics now call the component the native
host. The root dependency check remains useful, but no longer depends on `jq`
or points readers to the former monorepo repository.

Validation:

- the maintained Python locks passed `uv lock --check`
- the root dependency policy check passed from outside the repository root
- unarchived source has no executable references to the retired installer,
  version checker, verifier scripts, or link-handler binary
- remaining `system-bridge` references are confined to archived history and
  this execution record
- workflow YAML parsing, documentation checks, formatting, and
  `git diff --check` passed

### Release Asset Integrity

The Tauri release finalizer now builds `SHA256SUMS` from the SHA-256 digests
GitHub records as each asset is uploaded, fails if any asset lacks a valid
digest, and attaches the manifest to the release. This avoids redownloading
the full cross-platform asset set merely to recompute the same hashes. The
release download table links the resulting manifest.

Both supported Linux installers download the manifest first, require an entry
for the exact selected asset, hash the downloaded bytes locally, and delete
the download on a mismatch. Release 0.2.1 predates the new finalizer, so its
GitHub-recorded asset digests are also published as a website-hosted bootstrap
manifest. Later releases use their attached manifest. Missing both manifests,
a missing entry, malformed digest, missing `sha256sum`, failed asset download,
or mismatched digest aborts before any installation step.

The shell test gate sources the exact verification function from each public
installer and covers a successful download, tampered bytes, and an unavailable
manifest, bootstrap fallback, or missing asset entry. The repository sanity
workflow runs this test on every push and pull request.

Validation:

- all ten installer integrity cases passed across the two public installers
- the bootstrap manifest matched all 17 digests currently recorded by GitHub
  for desktop release 0.2.1
- the rendered finalizer step generated the same manifest from live release
  metadata and reached a mock upload successfully
- the website build included the bootstrap manifest byte-for-byte
- workflow YAML parsing, documentation checks, formatting, and shell syntax
  checks passed

### Final Local Validation

The final repeatable gate ran on Node 24.18.0 and pnpm 11.16.0 with a frozen
lockfile:

- ESLint completed with no errors and 41 existing warning-only findings;
  Prettier, active-documentation validation, and all TypeScript checks passed
- 2,109 JavaScript tests passed and one test remained intentionally skipped
- every workspace build passed, including the extension, Tauri frontend,
  engine CLI, plugin SDK, and Astro website
- `pnpm audit --prod` reported no known vulnerabilities
- Android compilation, all debug unit-test tasks, and lint completed
  successfully; the earlier emulator-backed transfer gate also passed
- Rust formatting, clippy with warnings denied, and all 105 workspace tests
  passed
- `cargo audit` reported no vulnerabilities and the 18 previously classified
  warning-only records in Tauri's Linux/platform transitive graph
- both maintained uv locks were immutable, both hashed Python audits reported
  no known vulnerabilities, and all 19 self-contained integration scenarios
  passed with two external-seeder cases intentionally skipped
- all ten release-integrity cases passed across the two public installers
- root dependency policy, workflow YAML parsing, and `git diff --check` passed

Android still emits the known Gradle 9 migration warning for deprecated Gradle
8 APIs. The JavaScript lint warnings and Rust warning-only advisories predate
this campaign and are visible in their respective tools; none is a failing
security advisory or regression introduced by these updates.
