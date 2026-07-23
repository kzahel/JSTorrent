# iOS AltStore PAL Distribution

Topic: `ios-altstore-pal-distribution`

Status: **current operational release record, reconciled with the release
scripts and workflows on 2026-07-23**.

JSTorrent iOS is distributed through AltStore PAL in the EU. Apple rejects
torrent clients from the App Store under guideline 5.2.3, so the release must
use Apple's alternative-distribution notarization flow rather than App Store
review.

## Release Invariants

- Bundle ID: `com.jstorrent.ios`
- Release tags use `ios-v<version>`.
- App Store Connect versions must use `reviewType: NOTARIZATION`.
- Never click **Submit for Review** in App Store Connect. That starts full App
  Store review and has previously caused a guideline 5.2.3 rejection.
- Signing keys, API keys, certificates, profiles, and short-lived AltStore
  tokens stay in secret storage. Do not add their values or maintainer-local
  paths to this document.
- The App Store Connect API key needs the Admin role to create versions and
  submit them for notarization. A Developer-role key can upload a build but
  cannot finish this flow.

## Normal Release Flow

1. Add a `## [<version>]` entry to
   [`ios/CHANGELOG.md`](../../ios/CHANGELOG.md).
2. Start from a clean working tree and run:

   ```bash
   ./scripts/release-ios.sh <version>
   ```

3. The release script increments the Xcode build number, commits and pushes the
   version change, then creates and pushes the `ios-v<version>` tag.
4. [`ios-ci.yml`](../../.github/workflows/ios-ci.yml) builds and signs the IPA,
   uploads it to App Store Connect, and invokes
   [`fetch-adp.py`](../../ios/scripts/fetch-adp.py).
5. `fetch-adp.py` creates or reuses the version with
   `reviewType: NOTARIZATION`, resolves conflicting old submissions, submits
   the build, and waits up to three hours for the Alternative Distribution
   Package (ADP).
6. CI attaches the IPA and `adp.tar.gz` to a draft GitHub release, generates
   [`website/public/altstore-source.json`](../../website/public/altstore-source.json),
   commits that generated source to `main`, and publishes the release.

The ADP contains Apple's manifest, signature, and device variants. Do not alter
its contents.

## Verification

- Confirm the `ios-v<version>` GitHub Actions run completed.
- Confirm the corresponding GitHub release is no longer a draft and includes
  the IPA and `adp.tar.gz`.
- Confirm <https://jstorrent.com/altstore-source.json> names the new version.
- The user-facing source URL is
  `altstore://source?url=https://jstorrent.com/altstore-source.json`.

## Recovery

If the main workflow times out or stops after notarization, manually run the
**iOS Finalize Release** workflow with the version number. The fallback
workflow in
[`ios-finalize-release.yml`](../../.github/workflows/ios-finalize-release.yml)
fetches the ADP, replaces the release asset, regenerates the source JSON,
commits it, and publishes the release.

[`fetch-adp.py`](../../ios/scripts/fetch-adp.py) also handles versions claimed
by an older submission and can cancel conflicting submissions before retrying
with `reviewType: NOTARIZATION`.

| Symptom | Likely cause | Response |
| --- | --- | --- |
| Guideline 5.2.3 rejection | Full App Store review was started | Ignore or cancel that review and create a notarization submission via the automated flow |
| `ITEM_PART_OF_ANOTHER_SUBMISSION` | An older review owns the version | Let `fetch-adp.py` cancel and replace the conflicting submission |
| API request returns `403` | App Store Connect key lacks Admin access | Replace it with an Admin-role API key |
| ADP is not available yet | Notarization is still running | Allow the workflow's three-hour polling window, then use the fallback workflow |
| AltStore API authentication fails | Short-lived PAL registration token expired | Re-register using the AltStore API without recording the token or account values in Git |

## CI Secrets

The workflows expect these repository secrets:

- `ASC_API_KEY_P8_BASE64`
- `ASC_API_KEY_ID`
- `ASC_API_ISSUER_ID`
- `IOS_CERTIFICATE_P12_BASE64`
- `IOS_CERTIFICATE_PASSWORD`
- `IOS_PROVISIONING_PROFILE_BASE64`
- `MACOS_KEYCHAIN_PASSWORD`

`GITHUB_TOKEN` is supplied by GitHub Actions.

## Code Map

- [`scripts/release-ios.sh`](../../scripts/release-ios.sh): validates, versions,
  commits, pushes, and tags a release.
- [`ios-ci.yml`](../../.github/workflows/ios-ci.yml): primary build,
  notarization, ADP, and publication workflow.
- [`ios-finalize-release.yml`](../../.github/workflows/ios-finalize-release.yml):
  manual recovery workflow.
- [`ios/scripts/fetch-adp.py`](../../ios/scripts/fetch-adp.py): App Store
  Connect submission and ADP retrieval.
- [`scripts/ios-finalize-release.sh`](../../scripts/ios-finalize-release.sh):
  generates the published AltStore source JSON.
- [`ios/altstore-source.template.json`](../../ios/altstore-source.template.json):
  source template.

## History

- 2026-03-14: Alternative distribution account and marketplace setup completed.
- 2026-03-20: A manual App Store review submission demonstrated why the
  notarization distinction must remain explicit.
- 2026-03-23: The notarization and ADP publication path was automated and
  tested end to end.
- 2026-07-23: The two overlapping iOS guides were reconciled with the current
  three-hour CI flow and consolidated into this topic.
