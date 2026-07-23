# Repository Scripts

These scripts operate across workspace boundaries. Run them from the repository
root unless their usage output says otherwise.

## Validation and Test Support

- `check.sh`: the primary local lint, type, documentation, and test gate
- `check-root-deps.sh`: ensures runtime dependencies stay with their workspace
- `check-docs.mjs`: active documentation validation
- `test-release-integrity.sh`: checksum verification tests for public installers
- `e2e-companion-smoke.sh`: extension-to-Android companion download smoke test
- `setup-android-test-env.sh`: Android test dependencies
- `test-dht-*.ts` / `.cjs`: targeted DHT diagnostics
- `test-internet-archive-plugin.ts`: search-plugin diagnostic
- `benchmark-*.sh`: daemon and extension throughput measurements

## Release Scripts

- `release-engine.sh`
- `release-plugin-sdk.sh`
- `release-extension.sh`
- `release-android.sh`
- `release-tauri-app.sh`
- `release-ios.sh`
- `release-website.sh`

Release scripts are state-changing: most edit version files, commit, push, and
create a tag. Do not run one merely to inspect it. Read
[`docs/topics/releases.md`](../docs/topics/releases.md) first.

## Deployment and Packaging

- `package-extension.sh`: Chrome Web Store ZIP
- `build-playstore-bundle.sh`: local signed Android AAB recovery path
- `deploy-chromebook.sh`: ChromeOS extension deployment
- `deploy-android-chromebook.sh`: ChromeOS Android deployment
- `deploy-windows-vm.sh`: Windows test deployment
- `linux-install-native-host-from-local-curl.sh`: local Crostini install test
- `update-android-web-assets.sh`: regenerate Android-hosted web assets
- `vite-search-plugin-sandbox-assets.mjs`: shared sandbox build helper

Legacy-app scripts remain for the archived Chrome App and are not part of the
current release pipeline.
