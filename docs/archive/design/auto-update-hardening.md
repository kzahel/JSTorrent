# Auto-Update Hardening

The nightmare scenario: we release a Tauri desktop app version where the auto-update mechanism is broken. Every user who upgrades to that version is permanently stuck — they can never auto-update again. This document catalogs the failure modes and the testing strategy to prevent them.

## Current State

**Zero automated test coverage** for the auto-updater. The CI workflow (`tauri-app-ci.yml`) builds the app but:
- Never runs `cargo test`
- Never validates the updater config
- Never verifies the `latest.json` artifact schema
- Never checks signing key consistency

## Architecture Summary

The auto-update system has several code paths:

| Path | Trigger | Code |
|------|---------|------|
| **Frontend startup check** | 5s after app launch | `desktop/tauri-app/src/updater.ts` → `check()` |
| **Frontend periodic check** | Every 24 hours | Same file, `setInterval` |
| **Frontend manual check** | Tray menu "Check for Updates" | Same file, listens for `check-for-updates` event |
| **Headless check** | Native host spawns app with `--check-update` | `desktop/tauri-app/src-tauri/src/headless_updater.rs` |
| **Headless auto-install** | Native host spawns app with `--auto-update` | Same file, downloads + installs + restarts |
| **Host direct HTTP check** | Extension requests check via native messaging | `desktop/host/src/updater.rs` → `check_for_updates_http()` |

The frontend path (startup check) is the most critical — it's how standalone Tauri users discover updates.

### Update endpoint

`https://updates.jstorrent.com/tauri/{target}/{arch}/{current_version}`

Returns HTTP 200 + JSON (update available) or HTTP 204 (up to date).

### Signature verification

Updates are signed with minisign (EdDSA). The public key is embedded in `tauri.conf.json` under `plugins.updater.pubkey`. The private key is in GitHub Secrets (`TAURI_SIGNING_PRIVATE_KEY`). If these don't match, downloads succeed but installation is rejected.

## Failure Scenarios

### 1. `tauri.conf.json` updater config accidentally broken

**Likelihood: Medium** — this file gets edited for bundle settings, window config, etc.

**How it breaks:**
- Endpoint URL template loses `{{target}}`, `{{arch}}`, or `{{current_version}}` placeholders
- `pubkey` is corrupted or removed
- `plugins.updater` section is accidentally deleted
- `createUpdaterArtifacts` is set to `false`

**Impact:** Complete — no user on this version can ever auto-update.

**Detection:** None today. Would only be caught by a human manually testing.

### 2. Tauri plugin version bump breaks the `check()` API

**Likelihood: Medium** — happens during routine dependency updates.

**How it breaks:**
- `@tauri-apps/plugin-updater` JS API signature changes
- `tauri-plugin-updater` Rust crate changes response parsing or signature validation
- Plugin initialization API changes (`.header()`, `.build()`)

**Impact:** Silent failure. `check()` throws, caught by `try/catch` in `updater.ts:37`, logged to console. User never sees an update prompt. On startup/periodic checks, the error is swallowed entirely.

**Detection:** None. TypeScript type-checking catches API changes in the JS side, but Rust plugin behavior changes (e.g., stricter signature validation) are invisible until runtime.

### 3. `initUpdater()` call removed or unreachable

**Likelihood: Low-Medium** — refactoring `main.tsx` or the app setup.

**How it breaks:**
- Import removed during refactoring
- An error earlier in startup prevents `initUpdater()` from being reached
- Conditional logic accidentally gates it

**Impact:** Complete — no startup check, no periodic check, no tray menu response. Totally dead updater with zero user-visible indication.

**Detection:** None.

### 4. `--check-update` / `--auto-update` CLI path breaks

**Likelihood: Low-Medium** — the arg parsing is fragile string comparison.

**How it breaks:**
- Someone switches from manual arg parsing to `clap` and the flag names change
- The `if check_update || auto_update { headless_updater::run(...); return; }` block gets moved or removed
- The headless updater fails to build the minimal Tauri app (e.g., context generation changes)

**Impact:** Extension users never get update notifications. The native host spawns the Tauri app with `--check-update`, but instead of running headless, it opens a normal window and never writes the result file. The host times out after 60 seconds.

**Detection:** None.

### 5. `latest.json` not generated or wrong schema

**Likelihood: Low** — but catastrophic if it happens.

**How it breaks:**
- CI condition for `includeUpdaterJson` changes (currently gated on `startsWith(github.ref, 'refs/tags/')`)
- Tauri action version bump changes the `latest.json` schema
- `finalize-release` job accidentally deletes it along with `.sig` files

**Impact:** The update endpoint serves stale or missing data. Existing users never see new updates.

**Detection:** None. The `finalize-release` job deletes `.sig` files by pattern — if `latest.json` matched a glob, it would be silently deleted.

### 6. Signing key mismatch

**Likelihood: Low** — but very confusing when it happens.

**How it breaks:**
- `TAURI_SIGNING_PRIVATE_KEY` secret is rotated but `pubkey` in `tauri.conf.json` isn't updated
- Key is regenerated due to a security incident
- Copy-paste error when updating secrets

**Impact:** Users see "Update Available", click "Install & Restart", it downloads 100%, then fails with a cryptic signature verification error. They may retry repeatedly. This is actually *worse* than a totally broken updater because it actively degrades the user experience while providing no path forward.

**Detection:** None until a user reports it.

### 7. Update server (`updates.jstorrent.com`) misconfigured

**Likelihood: Low-Medium** — not a code regression, but a deploy/infra issue.

**How it breaks:**
- DNS misconfiguration
- CDN/proxy changes break the URL routing for `/{target}/{arch}/{version}`
- SSL certificate expires
- Server starts returning HTML error pages instead of JSON/204

**Impact:** All users stop receiving update checks. Silent on startup/periodic; shows error on manual check.

**Detection:** None from CI. Would need external monitoring.

### 8. Version string format change breaks semver comparison

**Likelihood: Low** — but tricky.

**How it breaks:**
- Version changes from `0.1.x` to `1.0.0` and the update endpoint's version comparison logic doesn't handle the major version bump
- Pre-release suffixes (e.g., `1.0.0-beta.1`) confuse the comparator
- The Tauri plugin's internal semver comparison has edge cases

**Impact:** Users on old versions either never see the update, or users on the new version incorrectly see "update available" for an older version.

**Detection:** None.

## Testing Strategy

### Tier 1: Must-Have (CI gate for every release)

These are pure code/config tests — no infrastructure needed. They should block the release pipeline.

#### A. Config Validation Test (Rust)

Add a `#[test]` in the tauri-app crate or a CI script that:

1. Parses `tauri.conf.json`
2. Asserts `plugins.updater.endpoints[0]` contains `{{target}}`, `{{arch}}`, and `{{current_version}}`
3. Asserts `plugins.updater.pubkey` is valid base64 and correct length for a minisign public key
4. Asserts `bundle.createUpdaterArtifacts` is `true` (or `"v2"`)

```rust
#[test]
fn updater_config_is_valid() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let updater = &conf["plugins"]["updater"];

    let endpoint = updater["endpoints"][0].as_str().unwrap();
    assert!(endpoint.contains("{{target}}"), "endpoint missing {{target}}");
    assert!(endpoint.contains("{{arch}}"), "endpoint missing {{arch}}");
    assert!(endpoint.contains("{{current_version}}"), "endpoint missing {{current_version}}");
    assert!(endpoint.starts_with("https://"), "endpoint must be HTTPS");

    let pubkey = updater["pubkey"].as_str().unwrap();
    let decoded = base64::decode(pubkey).expect("pubkey must be valid base64");
    assert!(decoded.len() > 32, "pubkey suspiciously short");
}
```

#### B. Headless Updater Smoke Test (integration test or CI script)

After building the Tauri app in CI, run:

```bash
./target/release/jstorrent-desktop --check-update
```

Assert:
- Exits within 30 seconds (currently the network will fail in CI, but the process should still exit cleanly)
- Writes a JSON file to `~/.config/jstorrent-native/update-check-result.json`
- The JSON is valid and has the expected schema (`available`, `error` fields)
- Exit code is 0 (even when no network — errors are written to the result file, not the exit code)

This catches scenarios #3 and #4 — if the headless path is broken, this test fails.

#### C. Add `cargo test --workspace` to CI

The CI workflow currently never runs `cargo test`. Add it as a step after building sidecars:

```yaml
- name: Run Rust tests
  working-directory: desktop
  run: cargo test --workspace
```

This is trivial and catches any Rust compilation issues in test code.

#### D. Post-Release `latest.json` Validation

Add a step in the `finalize-release` job:

```yaml
- name: Validate latest.json
  run: |
    TAG="${GITHUB_REF_NAME}"
    # Download latest.json from the release
    gh release download "$TAG" -p "latest.json" -D /tmp/
    # Validate structure
    python3 -c "
    import json, sys
    data = json.load(open('/tmp/latest.json'))
    assert 'version' in data, 'missing version'
    assert 'platforms' in data, 'missing platforms'
    platforms = data['platforms']
    for key in ['darwin-aarch64', 'darwin-x86_64', 'linux-x86_64', 'windows-x86_64']:
        if key in platforms:
            p = platforms[key]
            assert 'url' in p, f'{key} missing url'
            assert 'signature' in p, f'{key} missing signature'
            assert p['signature'], f'{key} signature is empty'
    print(f'latest.json valid: v{data[\"version\"]} with {len(platforms)} platforms')
    "
```

### Tier 2: High-Value (require some setup)

#### E. Mock Update Server Integration Test

Create a test that:

1. Starts a local HTTP server on a random port
2. Serves a canned update response (200 + JSON with version, url, signature)
3. Overrides the update endpoint via Tauri's test configuration
4. Runs `check()` and asserts the update is detected
5. Serves a 204 and asserts "up to date"
6. Serves malformed JSON and asserts graceful error handling
7. Serves a timeout (delay response) and asserts the check doesn't hang forever

This is the most thorough test for the core update-checking logic. The Tauri updater plugin supports custom endpoints, so this is feasible without modifying production code — just pass a different endpoint URL in a test config.

#### F. Entry Point Assertion

A build-time or CI check that verifies `initUpdater` is imported and called in the app's entry point:

```bash
# In CI:
grep -q "initUpdater()" desktop/tauri-app/src/main.tsx || {
  echo "FATAL: initUpdater() not called in main.tsx — auto-updates are disabled!"
  exit 1
}
```

Fragile, but catches the "accidentally removed during refactoring" scenario. Could also be a TypeScript test that imports the module and asserts the function exists.

#### G. Signing Keypair Consistency Check

In CI (only on tag builds where `TAURI_SIGNING_PRIVATE_KEY` is available):

1. Extract the pubkey from `tauri.conf.json`
2. Use the private key to sign a test payload
3. Verify the signature with the embedded public key
4. Assert verification passes

This catches scenario #6 before any user is affected. Requires the `minisign` CLI tool in CI.

### Tier 3: Gold Standard

#### H. Canary / Staging Update E2E Test

The ultimate protection:

1. Maintain a staging update endpoint (e.g., `updates.jstorrent.com/staging/tauri/...`)
2. Before each release, publish the new version to staging
3. Run a test instance of the *previous* version, pointed at staging
4. Verify it detects the update, downloads it, installs it, and the new version launches
5. Verify the *new* version then checks for updates and correctly sees "up to date"

This is the only test that exercises the complete end-to-end flow including signature verification, binary compatibility, and restart behavior. It's the most effort to set up but provides the highest confidence.

#### I. External Uptime Monitor

A scheduled job (GitHub Actions cron, or a service like UptimeRobot) that periodically:

1. Curls `https://updates.jstorrent.com/tauri/darwin/aarch64/0.0.0` (should always return 200)
2. Validates the response is valid JSON with the expected schema
3. Alerts if the endpoint is down or returns unexpected content

This catches scenario #7 independently of releases.

## Implementation Priority

| Priority | Item | Effort | Catches |
|----------|------|--------|---------|
| **P0** | A. Config validation test | 1 hour | #1 (config broken) |
| **P0** | C. `cargo test` in CI | 5 min | General Rust regressions |
| **P0** | F. Entry point assertion | 10 min | #3 (initUpdater removed) |
| **P1** | B. Headless smoke test | 2 hours | #3, #4 (CLI path broken) |
| **P1** | D. `latest.json` validation | 1 hour | #5 (artifact missing/malformed) |
| **P1** | G. Signing key check | 2 hours | #6 (key mismatch) |
| **P2** | E. Mock server integration test | 4 hours | #2 (plugin API change), #8 (version format) |
| **P3** | H. Canary E2E test | 1-2 days | Everything (full end-to-end) |
| **P3** | I. Uptime monitor | 1 hour | #7 (server down) |

## Key Files

| File | Role |
|------|------|
| `desktop/tauri-app/src-tauri/tauri.conf.json` | Updater config (endpoint, pubkey) |
| `desktop/tauri-app/src/updater.ts` | Frontend update UI and check scheduling |
| `desktop/tauri-app/src-tauri/src/headless_updater.rs` | Headless check/install for native host |
| `desktop/tauri-app/src-tauri/src/lib.rs` | App entry point, `--check-update` arg parsing, updater plugin init |
| `desktop/host/src/updater.rs` | Native host: HTTP check + spawns Tauri app |
| `.github/workflows/tauri-app-ci.yml` | CI pipeline (build, sign, publish) |
| `scripts/release-tauri-app.sh` | Release script (version bump, tag, push) |
