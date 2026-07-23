# iOS Localized UI Implementation Plan

## Goals

- Build the iOS app shell around a minimal native SwiftUI interface: torrent list first, torrent detail second.
- Keep runtime lifecycle simple for the App Store-compatible path: lazy startup, foreground-first, aggressive shutdown in background.
- Make i18n a first-class constraint from the first UI PR.
- Reuse Android string keys and translations as the canonical shared UI vocabulary wherever possible.

## Non-Goals For The First Pass

- Full parity with Android settings and background behaviors.
- Continuous background downloading on iOS.
- Shipping the sideload-only background-audio workaround.
- Rebuilding the iOS app around a large architecture framework.

## Runtime Policy

### App Store Minimal Policy

- Do not start the JS engine on app launch.
- Start the engine only when the user:
  - adds a torrent
  - imports a `.torrent` file
  - opens a torrent detail screen
  - taps pause or resume
  - explicitly requests refresh/live data
- When the app backgrounds, stop active UI polling immediately.
- If there is no user-visible reason to keep the runtime alive, fully tear down the JS context instead of merely suspending it.
- If all torrents are complete or stopped, background transition should always end in a fully torn-down runtime.

### Sideload Experimental Policy

- Reserve a separate runtime-policy switch for future sideload-only behaviors.
- Keep the policy abstraction in place early, but do not implement the background-audio path in the MVP.

## Localization Strategy

### Canonical Source

- Android `strings.xml` remains the canonical shared source for UI string keys and translations.
- iOS should not invent a second naming system for equivalent UI text.

### Resource Format For PR 1

- Generate iOS bundle resources from Android `values*/strings.xml`.
- Use JSON dictionaries per locale copied into the iOS app bundle under `Resources/Localization/`.
- Add a lightweight Swift localization wrapper to read those dictionaries using `Locale.preferredLanguages`.

### Why JSON Instead Of Native `.strings` In PR 1

- The current Xcode project has no localization scaffolding.
- Directly syncing Android XML into iOS `.strings` variant groups is higher-friction and harder to automate cleanly in this repo.
- JSON resources keep Android as the source of truth and avoid an iOS-only translation maintenance path.

### Rules

- No user-facing Swift string literals in the torrent UI.
- Missing translations in non-English locales fall back to base English generated from Android `values/strings.xml`.
- New shared UI strings should be added to Android base strings first, then synced to iOS.

## UI Scope

### Phase 1: Existing Shell Cleanup

- Replace hardcoded strings in the current `ContentView`.
- Add localization wrapper and Android-to-iOS sync script.
- Keep the current single-screen layout functional while removing hardcoded copy.

### Phase 2: Main Screen Refactor

- Introduce a dedicated `TorrentListScreen`.
- Add minimal list states:
  - loading
  - empty
  - error
  - loaded
- Add toolbar-driven add/import affordances.
- Add simple filter chips or segmented control:
  - All
  - Active
  - Finished

### Phase 3: Navigation And Detail Shell

- Introduce `NavigationStack` routing to a torrent detail screen.
- Add detail sections:
  - Overview
  - Files
  - Trackers
  - Peers
  - Pieces
- Ship section shells early even if some data is initially placeholder-backed.

### Phase 4: Detail Data Bridge

- Extend `JSTorrentKit` to expose the data needed for detail sections:
  - files
  - trackers
  - peers
  - pieces
  - details
- Mirror Android’s data model shape where practical to keep the UI mental model aligned.

## Architecture

### App Layer

- `AppModel` or `RuntimeManager` owns the runtime lifecycle policy.
- `TorrentListViewModel` owns list presentation state.
- `TorrentDetailViewModel` owns per-torrent detail state.
- `JSTorrentKit` remains the runtime bridge and engine API surface, not the place for app-specific screen state.

### Localization Layer

- `sync-android-localizations` script:
  - reads Android `values*/strings.xml`
  - converts Android placeholder formats where needed
  - emits per-locale JSON files into the iOS resource folder
- `L10n` Swift wrapper:
  - resolves best locale match
  - reads generated JSON
  - falls back to English
  - optionally provides formatted-string helpers

## Delivery Sequence

### PR 1: Localization Scaffold

- Add shared-string additions to Android base strings for the current iOS shell where needed.
- Add Android-to-iOS localization sync script.
- Generate iOS localization resource files from Android strings.
- Add Swift localization wrapper.
- Replace hardcoded strings in the current iOS shell.

### PR 2: Lazy Runtime Manager

- Remove eager engine startup on app foreground.
- Add on-demand runtime startup triggers.
- Add explicit background teardown behavior.

### PR 3: List Screen Refactor

- Split `ContentView` into a dedicated list screen and supporting components.
- Add list states and navigation hooks.

### PR 4: Detail Screen Shell

- Add push navigation from list to detail.
- Add section tabs and localized placeholders.

### PR 5: Detail Bridge

- Extend `JSTorrentKit` models and subscriptions for detail sections.
- Wire data into the detail screen.

## Verification

### PR 1

- iOS app bundle contains generated localization resources.
- Current SwiftUI shell renders without hardcoded user-facing strings.
- Locale selection falls back correctly to English.
- Sync script runs deterministically from repo state.

### Runtime Work

- App launch does not initialize the JS engine.
- Engine starts only from user action or explicit live-data triggers.
- Backgrounding with no active user-visible work tears down the runtime.

## Follow-Up

- Once the localization resource format and key strategy prove stable, we can decide whether staying on custom JSON is sufficient or whether there is enough value in converting the generated output to native Apple localization catalogs later.
