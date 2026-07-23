# Android Standalone Search Plugins MVP

**Goal:** Add a slimmed-down search plugin experience to Android standalone with native Compose UI and a hidden `WebView` sandbox host. Users should be able to install plugins from URL, enable/disable/remove them in Settings, search from the main app, view results, and add torrents. The Plugin Lab and source editor are explicitly out of scope.

## Product Shape

### In scope

- Search entry point from the main torrent list top bar
- Dedicated in-app search screen
- Search plugin management from Settings
- Install plugin from URL
- Recommended one-tap install for Internet Archive
- Enable / disable / remove installed plugins
- Run search across enabled plugins
- Show results
- Add result via magnet or downloaded `.torrent`

### Out of scope

- Plugin Lab
- Source editing
- Request trace / debug console UI
- Draft-run UI
- Full desktop/extension parity
- QuickJS-based plugin runtime

## UX

### Main app entry points

#### 1. Search button in torrent list top bar

Add a search icon to the top app bar in [TorrentListScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentListScreen.kt). Tapping it navigates to a dedicated search route.

#### 2. Search Plugins section in Settings

Add a new settings row in [SettingsScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/SettingsScreen.kt) called `Search Plugins`.

### Search screen behavior

The search screen should be a Compose route, not a separate Android `Activity`. The app already uses a single navigation host in [Navigation.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/navigation/Navigation.kt), so adding routes is the lowest-friction path.

#### Normal state

- Search query field
- Optional category selector
- Search button
- Results list
- Add button on each result

#### Empty state: no enabled plugins

Show:

- Message: no search plugins are enabled
- CTA: `Install Internet Archive`
- CTA: `Manage Search Plugins`

#### Empty state: no results

Show a simple no-results message, preserving the installed plugin state and search form.

### Search Plugins settings screen behavior

Show:

- Recommended plugin card for Internet Archive
- Installed plugins list
- Enable / disable toggle per plugin
- Remove action
- Add-from-URL field and install button

Do not show:

- Source code
- Plugin editing
- Sandbox trace details

## Why Hidden WebView First

The shared sandbox runtime already assumes browser APIs such as `DOMParser` and a message-based host bridge. Reusing that shape with a hidden Android `WebView` minimizes new platform code and keeps plugin compatibility high.

This MVP should reuse:

- Shared sandbox assets in [packages/client/search-plugin-sandbox](/Users/kgraehl/code/jstorrent/packages/client/search-plugin-sandbox)
- Existing plugin manifest / installed plugin record structure from `packages/client/src/search/types.ts`
- Existing fetch mediation rules and allowed-host enforcement model

This MVP should not attempt to port plugin execution to QuickJS yet. QuickJS can remain a future follow-up behind the same host interface if desired.

## Architecture

### High-level flow

1. User taps search icon in torrent list.
2. App navigates to `Routes.SEARCH`.
3. Search screen loads enabled plugins from repository.
4. If no enabled plugins exist, show empty state with install shortcut.
5. When search runs, the native host sends plugin source + input into a hidden `WebView`.
6. The sandbox requests network fetches through a narrow host bridge.
7. Native host enforces allowed hosts, timeouts, redirect rules, and response size caps.
8. Search results are returned to the ViewModel and rendered in Compose.
9. Tapping `Add` sends the magnet URL or downloaded torrent bytes into the existing add-torrent flow.

## New Android Components

### 1. `SearchPluginRepository`

**Responsibility:** persistence and install/update/list/remove operations for search plugins.

**Suggested file:**
- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginRepository.kt`

**Responsibilities:**

- Store installed plugin records as JSON
- Return installed plugins sorted by name
- Save enabled/disabled state
- Remove plugin by ID
- Provide recommended plugin metadata for Internet Archive

**Storage choice:**

- Use `SharedPreferences` or a small dedicated JSON file for MVP
- Do not mix plugin records into `SettingsStore` unless the stored shape is tiny and stable

Recommended approach:

- Keep `SettingsStore` for simple scalar Android settings
- Use a dedicated repository-backed store for plugin records

### 2. `AndroidSearchPluginSandboxHost`

**Responsibility:** own a hidden `WebView` and implement the shared plugin sandbox protocol.

**Suggested files:**

- `android/app/src/main/java/com/jstorrent/app/search/AndroidSearchPluginSandboxHost.kt`
- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginWebViewBridge.kt`

**Responsibilities:**

- Create and retain a hidden `WebView`
- Load `search-plugin-sandbox.html` from app assets
- Run `inspectSource`
- Run `runDraft`
- Mediate `fetch-request` / `fetch-response`
- Dispose and recreate the `WebView` on fatal errors if needed

**Security requirements:**

- JavaScript enabled only for the hidden sandbox `WebView`
- No arbitrary JS interfaces besides the narrow bridge
- No file access
- No content access
- No direct network access from plugin code
- All fetches go through Kotlin host mediation

**WebView settings to lock down:**

- `javaScriptEnabled = true`
- `domStorageEnabled = false`
- `allowFileAccess = false`
- `allowContentAccess = false`
- `setSupportMultipleWindows(false)`
- block navigation away from the sandbox asset

### 3. `SearchPluginFetchMediator`

**Responsibility:** enforce plugin fetch policy from Android.

**Suggested file:**
- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt`

**Responsibilities:**

- Validate requested host against manifest allowlist
- Support `GET` and `POST` only
- Follow limited redirects
- Re-validate redirect target host
- Return text, bytes, final URL, status code
- Enforce size cap and timeout

### 4. `SearchPluginSettingsViewModel`

**Responsibility:** manage the settings screen state.

**Suggested file:**
- `android/app/src/main/java/com/jstorrent/app/viewmodel/SearchPluginSettingsViewModel.kt`

**State:**

- Installed plugins
- Add-from-URL field
- Busy state
- Error / status messages

### 5. `SearchViewModel`

**Responsibility:** manage the search screen state.

**Suggested file:**
- `android/app/src/main/java/com/jstorrent/app/viewmodel/SearchViewModel.kt`

**State:**

- Query
- Category
- Enabled plugin list
- Search-in-progress
- Search results
- Empty-state actions
- Add-result busy state

### 6. Compose screens

**Suggested files:**

- `android/app/src/main/java/com/jstorrent/app/ui/screens/SearchScreen.kt`
- `android/app/src/main/java/com/jstorrent/app/ui/screens/SearchPluginSettingsScreen.kt`

## Navigation Changes

Modify [Navigation.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/navigation/Navigation.kt).

### New routes

```kotlin
const val SEARCH = "search"
const val SETTINGS_SEARCH_PLUGINS = "settings/search_plugins"
```

### Route wiring

#### Torrent list screen

Add:

- `onSearchClick = { navController.navigate(Routes.SEARCH) }`

#### Settings hub screen

Add:

- `onNavigateToSearchPlugins = { navController.navigate(Routes.SETTINGS_SEARCH_PLUGINS) }`

#### New composables

- `composable(Routes.SEARCH) { SearchScreen(...) }`
- `composable(Routes.SETTINGS_SEARCH_PLUGINS) { SearchPluginSettingsScreen(...) }`

## UI Changes

### Torrent list top bar

Modify [TorrentListScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentListScreen.kt).

Add a search icon in the top app bar `actions` before the overflow menu.

New callback:

```kotlin
onSearchClick: () -> Unit = {}
```

### Settings hub

Modify [SettingsScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/SettingsScreen.kt).

Add a row:

- title: `Search Plugins`
- subtitle: `Manage search providers for in-app torrent search`

### Strings

Add new strings in [strings.xml](/Users/kgraehl/code/jstorrent/android/app/src/main/res/values/strings.xml) for:

- Search
- Search Plugins
- Install Internet Archive
- Manage Search Plugins
- Add from URL
- No plugins enabled
- No search results
- Search results
- Install / Remove / Enable / Disable

## Shared Asset Packaging

Android must package the shared sandbox assets from [packages/client/search-plugin-sandbox](/Users/kgraehl/code/jstorrent/packages/client/search-plugin-sandbox).

### Recommended approach

Add a Gradle task in `android/app/build.gradle.kts` to copy:

- `search-plugin-sandbox.html`
- `search-plugin-sandbox.js`

into:

- `android/app/src/main/assets/search-plugin-sandbox/`

This mirrors the current shared-asset approach used for Tauri and the extension while keeping Android independent at runtime.

### Runtime URL

The hidden `WebView` should load:

- `file:///android_asset/search-plugin-sandbox/search-plugin-sandbox.html`

## Add Torrent Integration

Reuse the existing standalone torrent add path in [TorrentListViewModel.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/viewmodel/TorrentListViewModel.kt#L395) and the underlying repository.

### Behavior

#### Magnet result

- Pass magnet URI directly to `addTorrent(...)`

#### `.torrent` result

- Fetch the URL through `SearchPluginFetchMediator`
- Validate response
- Base64-encode the torrent bytes
- Pass the encoded bytes to `addTorrent(...)`

This keeps search result add behavior aligned with the app’s existing input path.

## Security Model

This MVP should preserve the same security shape as extension/Tauri:

- Plugin code does not get direct engine access
- Plugin code does not get direct storage access
- Plugin code does not get raw Android API access
- Plugin code does not get unrestricted network access

### Host-exposed plugin API

Keep the exposed runtime surface limited to:

- `encode`
- `fetchText`
- `fetchJson`
- `parseHtml`
- `emitResult`
- `log`

### Host-side restrictions

- Allowed-host enforcement from plugin manifest
- Request timeout
- Response size limit
- Redirect limit
- Results-per-plugin cap
- String length cap for logs/results if needed

## Implementation Phases

### Phase 1: Data + host plumbing

**Goal:** make plugin storage and hidden `WebView` execution work without UI polish.

**Files to create:**

- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginRepository.kt`
- `android/app/src/main/java/com/jstorrent/app/search/AndroidSearchPluginSandboxHost.kt`
- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt`

**Files to modify:**

- `android/app/build.gradle.kts`

**Verification:**

- Can install Internet Archive plugin from URL
- Can persist plugin list across app restarts
- Can inspect source
- Can run search and get results in a test harness

### Phase 2: Settings management UI

**Goal:** plugin admin in Settings.

**Files to create:**

- `android/app/src/main/java/com/jstorrent/app/ui/screens/SearchPluginSettingsScreen.kt`
- `android/app/src/main/java/com/jstorrent/app/viewmodel/SearchPluginSettingsViewModel.kt`

**Files to modify:**

- [Navigation.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/navigation/Navigation.kt)
- [SettingsScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/SettingsScreen.kt)
- [strings.xml](/Users/kgraehl/code/jstorrent/android/app/src/main/res/values/strings.xml)

**Verification:**

- Settings shows `Search Plugins`
- Recommended Internet Archive install works
- Enable / disable / remove works
- Add-from-URL works

### Phase 3: Search screen

**Goal:** user-facing search flow.

**Files to create:**

- `android/app/src/main/java/com/jstorrent/app/ui/screens/SearchScreen.kt`
- `android/app/src/main/java/com/jstorrent/app/viewmodel/SearchViewModel.kt`

**Files to modify:**

- [Navigation.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/navigation/Navigation.kt)
- [TorrentListScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentListScreen.kt)

**Verification:**

- Search icon appears in top bar
- Search route opens
- Empty state shows install shortcut when no enabled plugins exist
- Search results render
- Add result works

### Phase 4: Hardening + polish

**Goal:** make MVP safe and releaseable.

**Work:**

- Add timeouts and caps
- Improve error strings
- Add loading / empty / error states
- Ensure `WebView` lifecycle cleanup is correct
- Ensure plugin search cannot leak or hold activity references

**Verification:**

- App survives malformed plugin source
- App survives plugin fetch failure
- App survives plugin returning no results
- Repeated searches do not leak memory or duplicate `WebView` instances

## Tests

### Unit tests

Add JVM tests for:

- plugin repository persistence
- host allowlist validation
- search result sorting / aggregation
- add-result conversion logic

Suggested files:

- `android/app/src/test/java/com/jstorrent/app/search/SearchPluginRepositoryTest.kt`
- `android/app/src/test/java/com/jstorrent/app/search/SearchPluginFetchMediatorTest.kt`
- `android/app/src/test/java/com/jstorrent/app/viewmodel/SearchViewModelTest.kt`
- `android/app/src/test/java/com/jstorrent/app/viewmodel/SearchPluginSettingsViewModelTest.kt`

### Instrumented tests

Add Android tests for:

- search settings screen
- search screen empty state
- Internet Archive install shortcut
- add-result button flow

Suggested files:

- `android/app/src/androidTest/java/com/jstorrent/app/ui/screens/SearchScreenTest.kt`
- `android/app/src/androidTest/java/com/jstorrent/app/ui/screens/SearchPluginSettingsScreenTest.kt`

## Concrete File List

### New docs

- `docs/plans/android-search-plugins-mvp.md`

### New Android app files

- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginRepository.kt`
- `android/app/src/main/java/com/jstorrent/app/search/AndroidSearchPluginSandboxHost.kt`
- `android/app/src/main/java/com/jstorrent/app/search/SearchPluginFetchMediator.kt`
- `android/app/src/main/java/com/jstorrent/app/viewmodel/SearchViewModel.kt`
- `android/app/src/main/java/com/jstorrent/app/viewmodel/SearchPluginSettingsViewModel.kt`
- `android/app/src/main/java/com/jstorrent/app/ui/screens/SearchScreen.kt`
- `android/app/src/main/java/com/jstorrent/app/ui/screens/SearchPluginSettingsScreen.kt`

### Existing Android files to modify

- `android/app/build.gradle.kts`
- [Navigation.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/navigation/Navigation.kt)
- [TorrentListScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/TorrentListScreen.kt)
- [SettingsScreen.kt](/Users/kgraehl/code/jstorrent/android/app/src/main/java/com/jstorrent/app/ui/screens/SettingsScreen.kt)
- [strings.xml](/Users/kgraehl/code/jstorrent/android/app/src/main/res/values/strings.xml)

## Recommended First Slice

The first implementation slice should be:

1. Package shared sandbox assets into Android app assets
2. Implement `SearchPluginRepository`
3. Implement `AndroidSearchPluginSandboxHost`
4. Add a temporary test-only button or local screen to run Internet Archive search

Do not start with the full UI. Prove the hidden `WebView` host works first.

Once that is stable:

5. Add Settings management screen
6. Add Search route and top-bar search icon
7. Wire add-result into existing torrent add flow

## Deferred Follow-ups

- Replace hidden `WebView` with QuickJS host behind the same interface
- Add more recommended first-party plugins
- Add plugin update checks
- Add result filters / sort UI
- Add per-plugin status details in search screen

