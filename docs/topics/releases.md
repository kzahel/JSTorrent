# Releases

Topic: `releases`

Status: **current release map, reconciled with repository scripts and GitHub
Actions workflows on 2026-07-23**.

JSTorrent components release independently. Release scripts are not dry-run
helpers: most update version files, create a commit, push it, create a tag, and
push the tag. Run one only when a release is intended.

## Preflight

Before any component release:

1. Work on `main` and fetch the current remote state.
2. Confirm the working tree is clean and the intended commits are pushed or
   ready to be pushed by the release script.
3. Review the component changelog and add `## [<version>]` when the script
   requires it.
4. Run the relevant build, tests, and product smoke tests.
5. Confirm `git config user.name` and `git config user.email` identify the
   maintainer.
6. Inspect the release script and current workflow before relying on this
   topic if either has changed since the last reconciliation date.

Use versions without a leading `v`, for example:

```bash
./scripts/release-android.sh 1.2.3
```

## Component Map

| Component | Script and tag | Automated result | Remaining manual work |
| --- | --- | --- | --- |
| Engine | `release-engine.sh`, `engine-v*` | Tests, builds, publishes `@jstorrent/engine` to npm, creates GitHub release | Verify npm and release |
| Plugin SDK | `release-plugin-sdk.sh`, `plugin-sdk-v*` | Tests, builds, publishes `@jstorrent/plugin-sdk` to npm, creates GitHub release | Verify npm and release |
| Extension | `release-extension.sh`, `extension-v*` | Builds, runs Playwright, creates ZIP and GitHub release | Upload ZIP to Chrome Web Store |
| Android | `release-android.sh`, `android-v*` | Builds signed APK/AAB and mapping file, creates GitHub release | Upload AAB and mapping to Play Console, publish |
| Tauri desktop | `release-tauri-app.sh`, `tauri-app-v*` | Builds signed installers, updater metadata, and GitHub release | Verify installers and updater propagation |
| iOS | `release-ios.sh`, `ios-v*` | Builds/tests, uploads to App Store Connect, notarizes, fetches ADP, updates AltStore source, publishes release | Verify; use fallback workflow only on failure |
| Website | `release-website.sh`, `website-v*` | Deploys website through GitHub Pages workflow | Tag is optional bookkeeping; normal relevant `main` changes also deploy |

All release scripts live in [`scripts/`](../../scripts/README.md). The matching
workflows live under [`.github/workflows/`](../../.github/workflows/).

## Engine and Plugin SDK

Both npm releases require a matching changelog entry and update their package
version before committing and tagging:

```bash
./scripts/release-engine.sh <version>
./scripts/release-plugin-sdk.sh <version>
```

The workflows run typechecking, tests, a package build, and `npm pack
--dry-run` before publishing with provenance.

## Extension

Before releasing the extension:

- Run `./scripts/e2e-companion-smoke.sh`; the release script checks for a
  recent successful run.
- Review backend minimums in
  `packages/client/src/hooks/useSystemBridge.ts` when the extension depends on
  new daemon or native-host behavior.
- Release required Android and Tauri backends before increasing those
  minimums.

Then run:

```bash
./scripts/release-extension.sh <version>
```

CI creates `jstorrent-extension.zip` and a GitHub release. Chrome Web Store
submission is manual.

## Android

```bash
./scripts/release-android.sh <version>
```

The script increments `versionCode` and updates `versionName`. Android CI
builds and attaches the signed APK, AAB, and compressed mapping file. Upload
the CI-produced AAB to Play Console; `build-playstore-bundle.sh` is a local
recovery path, not a required second build after successful CI.

## Tauri Desktop

```bash
./scripts/release-tauri-app.sh <version>
```

The script updates the Tauri configuration, JavaScript package, Rust workspace
version, lockfile, and changelog. CI builds platform installers, signs
configured targets, validates `latest.json`, and updates the GitHub release
download table. The finalizer also publishes `SHA256SUMS` from GitHub's
recorded asset digests. The Linux desktop and Crostini installers hash the
downloaded asset locally and refuse to install when the manifests are missing,
do not name the selected asset, or do not match. A website-hosted manifest
bootstraps release 0.2.1, which predates the release finalizer; subsequent
releases carry their manifest as a release asset. Existing applications
consume the updater metadata.

## iOS

```bash
./scripts/release-ios.sh <version>
```

The iOS path is automated through notarization and AltStore source publication.
Its safety invariants, verification steps, and recovery workflow are maintained
in
[`ios-altstore-pal-distribution.md`](ios-altstore-pal-distribution.md).

## Website

Relevant `website/**` changes on `main` deploy automatically. An optional
version tag can be created with:

```bash
./scripts/release-website.sh <version>
```

The deploy workflow discovers the latest Tauri release so download links use
current desktop artifacts.

## Recovery Rules

- Do not rerun a release script until checking whether its commit or tag was
  already created.
- If CI failed after the tag was pushed, prefer rerunning the failed workflow
  or job over inventing a replacement tag.
- Do not move or recreate a published tag without explicit maintainer
  direction.
- iOS has a dedicated **iOS Finalize Release** fallback workflow for
  post-notarization failures.

## Code Map

- [`release-engine.sh`](../../scripts/release-engine.sh)
- [`release-plugin-sdk.sh`](../../scripts/release-plugin-sdk.sh)
- [`release-extension.sh`](../../scripts/release-extension.sh)
- [`release-android.sh`](../../scripts/release-android.sh)
- [`release-tauri-app.sh`](../../scripts/release-tauri-app.sh)
- [`release-ios.sh`](../../scripts/release-ios.sh)
- [`release-website.sh`](../../scripts/release-website.sh)
- [`engine-publish.yml`](../../.github/workflows/engine-publish.yml)
- [`plugin-sdk-publish.yml`](../../.github/workflows/plugin-sdk-publish.yml)
- [`extension-ci.yml`](../../.github/workflows/extension-ci.yml)
- [`android-ci.yml`](../../.github/workflows/android-ci.yml)
- [`tauri-app-ci.yml`](../../.github/workflows/tauri-app-ci.yml)
- [`ios-ci.yml`](../../.github/workflows/ios-ci.yml)
- [`ios-finalize-release.yml`](../../.github/workflows/ios-finalize-release.yml)
- [`deploy-website.yml`](../../.github/workflows/deploy-website.yml)
