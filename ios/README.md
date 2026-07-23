# JSTorrent for iOS

The iOS application is a native SwiftUI BitTorrent client. It embeds the shared
TypeScript engine in JavaScriptCore and implements file, TCP, UDP, hashing,
configuration, and application lifecycle services in `JSTorrentKit`.

The app targets iPhone and iPad on iOS 16 or newer. Its authoritative project
definition is [`project.yml`](project.yml), and the generated
`JSTorrent.xcodeproj` is not the source of truth.

## Prerequisites

- macOS with Xcode
- XcodeGen 2.38 or newer
- Node.js and pnpm for the TypeScript engine bundle

Install dependencies from the repository root before building:

```bash
pnpm install
```

## Simulator Workflow

The normal local workflow builds the native engine bundle, generates the Xcode
project, builds the app, installs it, and launches it:

```bash
ios/scripts/sim-start.sh
ios/scripts/sim-install.sh
```

Run `ios/scripts/sim-install.sh --help` for device, configuration, and
incremental-build options.

To perform the steps manually:

```bash
pnpm -C packages/engine bundle:native
ios/scripts/sync-engine-bundle.sh
xcodegen generate --spec ios/project.yml
```

## Tests

Run the Swift package tests:

```bash
swift test --package-path ios/JSTorrentKit
```

Run the application test target after generating the project:

```bash
xcodebuild \
  -project ios/JSTorrent.xcodeproj \
  -scheme JSTorrent \
  -configuration Debug \
  -destination 'platform=iOS Simulator,name=iPhone 16' \
  -derivedDataPath ios/build \
  CODE_SIGNING_ALLOWED=NO \
  test
```

The iOS CI workflow runs both test suites.

## Code Map

- `JSTorrent/App/`: SwiftUI application, screens, models, and search UI
- `JSTorrentKit/Sources/JSTorrentKit/`: JavaScriptCore engine controller and
  native bindings
- `JSTorrentKit/Tests/`: runtime, startup, search, and performance tests
- `JSTorrent/Resources/engine.bundle.js`: generated TypeScript engine bundle
- `scripts/sync-engine-bundle.sh`: copies the current native engine bundle
- `scripts/fetch-adp.py`: App Store Connect notarization and ADP retrieval

## Distribution

JSTorrent is distributed through AltStore PAL where Apple permits alternative
marketplaces, and can also be sideloaded. The release is automated after an
`ios-v<version>` tag.

Before releasing, read:

- [Repository release topic](../docs/topics/releases.md)
- [iOS AltStore PAL distribution topic](../docs/topics/ios-altstore-pal-distribution.md)

The release script commits, pushes, and tags the version:

```bash
./scripts/release-ios.sh <version>
```
