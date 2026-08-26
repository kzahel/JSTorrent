# 002: Legacy Packaged App Migration Messaging

Status: in progress.

Replace the paid and Lite Chrome packaged apps' obsolete waitlist notice with
an accurate, respectful path to the current JSTorrent product. Preserve
browser startup as the primary notification moment, bound reminders, and keep
the final Store packages independent of the replacement engine architecture.

Last reconciled: 2026-08-26.

## Desired Outcome

Existing packaged-app users receive a clear notification near the beginning
of a Chrome session rather than while they are doing unrelated work. The
notice explains that the legacy Chrome App no longer launches on current
ChromeOS and desktop Chrome, points to a stable JSTorrent-owned migration
page, and does not imply that the Chrome extension alone is the complete
replacement.

The campaign must cover both existing Store identities:

| Variant | Chrome Web Store ID | Store baseline on 2026-08-26 | Dashboard users on 2026-08-26 |
| --- | --- | --- | ---: |
| JSTorrent | `anhdpjpojoipgpmfanmedjghaligalgb` | `2.4.4` | 31,553 |
| JSTorrent Lite | `abmohcnlldaiaodkpacnldcdnjjgldfh` | `2.4.12` | 11,124 |

Candidate versions are expected to be `2.4.5` and `2.4.13`, respectively.
Reconcile the live Store baselines again before assigning versions or building
upload artifacts.

## Current Evidence

Exact packages fetched from Google's update service on 2026-08-26 contain the
same December 2025 migration implementation. That implementation:

- arms `migrationNoticePending` when an update is installed but shows no UI
  during `onInstalled`;
- calls the notice from `chrome.runtime.onStartup` and
  `chrome.app.runtime.onLaunched`;
- shows a persistent native notification titled `JSTorrent is Moving`;
- says that the replacement is not ready and offers `Join Waitlist` and
  `Remind Me Later`;
- shows again at every later browser startup after reminder or ordinary
  dismissal because it has no time-based throttle;
- permanently suppresses itself after the notification body or waitlist
  action is activated;
- does not reset the old dismissal when a later package update arrives; and
- points to `https://new.jstorrent.com/comingsoon.html`, whose GitHub Pages
  404 document uses JavaScript to preserve the path and redirect a browser to
  `https://jstorrent.com/comingsoon.html`.

The final browser destination now says JSTorrent is available, but it is not a
dedicated migration guide and adds an avoidable redirect and landing-page
hop. The current exact packages do not contain the richer migration window
present in the repository.

The code under `archive/legacy-app/` is not the Store baseline. It contains an
unshipped maximum-aggressiveness experiment that opens a notification and app
window on script load, startup, installation, launch, and a ten-minute alarm.
Do not package that tree without reconciling it against the exact Store
artifacts and replacing the experimental policy. The older
[`legacy-migration.md`](../archive/project/legacy-migration.md) records that
experiment and historical traffic evidence; it is not current product truth.

### Exact Store baseline reconciliation

The 2026-08-26 reconciliation downloaded both CRX3 artifacts directly from
Google's update service and recorded:

| Variant | Version | CRX SHA-256 | Extracted files |
| --- | --- | --- | ---: |
| Paid | `2.4.4` | `f94861ae34ae00f6a078cd17ad05ef9274cc96c1321d750cf9027c1c8a96636a` | 123 |
| Lite | `2.4.12` | `25dcba280f2d6f7e446fc8bea5071cc568d793c7c401600b0c0f3fcece99dd03` | 123 |

The two extracted products differ only in the manifest version, English
localized name (`JSTorrent` versus `JSTorrent Lite`), and Store-generated
`_metadata/verified_contents.json`. Every executable and content file is
otherwise byte-identical.

The paid Store artifact and `archive/legacy-app/` differ only in the final
migration section of `background.js`, the Store-generated `_metadata/`
directory, an accidentally shipped localization backup, the repository-only
packaging script, and the unshipped `migrate.html`/`migrate.js`. All source
before the migration section is byte-identical. Therefore:

- `archive/legacy-app/` remains the canonical editable source tree;
- the replacement migration implementation must be reviewed as a complete
  replacement for the experimental final section;
- paid and Lite are explicit packaging overlays on one source tree; and
- Store-generated metadata and accidental backup files are not inputs to the
  new packages.

The existing Store-signed CRX3 headers contain distinct public keys that
derive the expected paid and Lite IDs. Those public keys may be applied as
test-only manifest overlays so unpacked transition fixtures retain stable
identity. They are not shipping inputs, and no private Store key is required
or permitted.

## Accepted Product Decisions

### Startup is the primary delivery boundary

Browser startup is an appropriate, comparatively low-interruption time for
this notice. A real user observed and understood the existing notification
when powering on a Chromebook. Preserve that useful context instead of
copying Web Server for Chrome's immediate post-update notification policy.

The campaign uses these event semantics:

| Event or action | Required behavior |
| --- | --- |
| Migration update installed | Activate the new campaign and persist its state. Do not display migration UI immediately. |
| Browser/profile startup | Show the notification when the campaign is active and due. Record the prompt time when it is created. |
| Startup less than seven days after the last prompt | Show nothing. Do not create an alarm merely to prompt later in the session. |
| First startup after the seven-day interval | Show one reminder. The effective interval may be longer when the user does not restart Chrome. |
| Explicit legacy-app launch | Where Chrome still delivers `onLaunched`, show or focus the richer local migration window while preserving the existing legacy launch behavior. |
| Notification body or primary action | Open the stable migration page and mark this campaign acknowledged so it does not prompt again. |
| `Remind me in 7 days` | Snooze for seven days and clear the visible notification. |
| Notification dismissed by the user or platform | The recorded prompt time still prevents another notification for seven days. |
| Permanent stop action in the local migration window | Disable this campaign and clear its notification state. |
| Confirmed legacy-app removal | Let Chrome remove the app only through its native confirmation dialog. |

There is no script-load nag, repeating migration alarm, automatic migration
tab, or automatic migration app window. Startup is the only background
delivery moment. `onLaunched` remains an additive explicit-user path for older
runtimes; the current ChromeOS launcher is known not to deliver it.

### Campaign state is versioned and respectful

The old `migrationNoticeDismissed` value represented acknowledgment of a
waitlist campaign, not acknowledgment that JSTorrent is now available. Use a
new campaign identifier and campaign-scoped state so that the old dismissal
does not suppress this materially different notice.

The new state must distinguish at least:

- active campaign identifier;
- last prompted time;
- snoozed-until time;
- acknowledged time; and
- permanently disabled time.

State transitions must be idempotent across duplicate event delivery. A
future update within the same campaign must not reset acknowledgment or snooze
state. A future, genuinely different campaign may use a new identifier after
an explicit product decision; merely incrementing the app version is not
enough.

### Copy and destination are architecture-neutral

Use a stable owned route such as:

```text
https://jstorrent.com/migrate?ref=legacy-app-notification&variant=paid&platform=chromeos&campaign=available-2026
```

The exact campaign value is an implementation constant. `variant` and
`platform` are low-cardinality routing/analytics hints, not identifiers. The
page must remain correct when the replacement implementation changes from the
current TypeScript engine to RSTorrent.

Draft notification intent:

- Title: `JSTorrent has a new version`
- ChromeOS: explain that the old Chrome App no longer launches and that
  JSTorrent is available again; direct the user to setup options for this
  Chromebook.
- Windows/macOS/Linux: explain that Chrome no longer runs the old app and
  that JSTorrent is available for the user's desktop platform.
- Keep the visible short URL `jstorrent.com/migrate` in the body as a fallback
  when an operating system displays a notification but does not route its
  activation back to Chrome.
- Primary action: `See migration options`.
- Secondary action: `Remind me in 7 days`.

Final wording must fit the real ChromeOS and Windows notification renderings.
Do not say `Join the waitlist`, `coming soon`, `same features and more`, or
`you're all set` merely because the new extension responds to a ping.

The migration page must explain the current supported composition honestly:

- Windows, macOS, and Linux can install the standalone desktop application;
  the extension is optional browser integration.
- ChromeOS with Play support can use the Android application standalone, or
  pair the extension with the Android companion.
- ChromeOS without Play can pair the extension with the Crostini daemon.
- Detecting the new extension may tailor instructions, but it must not imply
  that the required native/Android/Crostini component is installed or working.
- The obsolete app may be removed independently through an explicit Chrome
  confirmation. Do not silently uninstall it or claim that user data will be
  transferred.

Keep `/comingsoon.html` and the `new.jstorrent.com` compatibility redirect
working for old packages already in the field. New packages and uninstall
URLs must link directly to the stable migration route.

## Scope

- Reconcile the paid and Lite Store packages against repository source.
- Implement shared campaign state, startup delivery, notification actions,
  platform copy, and a bounded local migration window.
- Add the stable website migration route and compatible query parameters.
- Produce independently versioned paid and Lite upload ZIPs from one reviewed
  source tree.
- Add automated behavior and package validation.
- Validate update and startup behavior on a physical Chromebook and a Windows
  VM through `~/code/machine-control`.
- Record exact-artifact hashes, device evidence, limitations, and deliberate
  deferrals in this tactical as work proceeds.

## Explicitly Out of Scope

- Changing or releasing the legacy helper extension
- Selecting or implementing the final JSTorrent-to-RSTorrent runtime contract
- Migrating torrent state, download roots, or incomplete files
- Claiming feature parity between the packaged app and any replacement
- Automatic removal of a packaged app without Chrome's native confirmation
- Treating August 31 as a Chrome packaged-app shutdown deadline
- Publishing either Chrome Web Store update without separate explicit user
  authorization
- Editing Chrome Web Store listing metadata except as a separately authorized
  follow-up

## Work Plan

### 1. Preserve and reconcile exact Store baselines

- [x] Fetch the current paid and Lite CRX artifacts through Google's update
  service, record their reported versions and SHA-256 hashes, and extract them
  only into ignored or temporary directories.
- [x] Diff both exact artifacts against each other and against
  `archive/legacy-app/`. Classify variant-specific name, version, configuration,
  and identity differences separately from accidental repository drift.
- [x] Treat the live package behavior as the starting point. Retain only
  reviewed parts of the unshipped migration experiment.
- [x] Establish a public-key-only unpacked-test identity strategy for both app
  IDs. Never retrieve, generate, copy, or commit a private Store signing key.
- [x] Record the resulting source-of-truth and variant model here before code
  changes proceed.

### 2. Implement and unit-test campaign state

- [x] Extract small ES-compatible pure helpers for platform classification,
  migration URL construction, notification copy, and reminder eligibility.
- [x] Replace the old pending/dismissed booleans with a campaign-scoped state
  transition layer.
- [x] Arm the campaign on the relevant `onInstalled` update without displaying
  a migration notification from that event.
- [x] Make `onStartup` the only automatic prompt trigger and enforce the
  seven-day minimum interval.
- [x] Ensure a notification close, reminder action, acknowledgment, permanent
  stop, duplicate event, and later update in the same campaign all have
  deterministic behavior.
- [x] Remove the experimental script-load prompt, startup window, ten-minute
  alarm, reason suffixes, and extension-only success copy.

Automated tests must cover:

- a user who dismissed the old waitlist campaign still sees the new campaign;
- a newly updated user sees nothing until startup;
- the first startup prompts and another startup within seven days does not;
- reminder eligibility resumes on the first startup after seven days;
- acknowledgment and permanent disable suppress later startup prompts;
- a package update within the same campaign preserves acknowledgment;
- paid/Lite and platform query parameters are bounded and correctly encoded;
- ChromeOS detection cannot be confused with ordinary Linux; and
- no automatic alarm or script-load prompt remains.

### 3. Implement notification and explicit-launch UI

- [x] Create the native notification with platform-aware copy, the stable
  migration URL, primary migration action, and seven-day reminder action.
- [x] Use `chrome.browser.openTab` where supported and a tested `window.open`
  fallback elsewhere.
- [x] Rework `migrate.html` and its script to show accurate platform guidance,
  optional extension detection, `See migration options`, `Remind me in 7
  days`, `Stop reminders`, and confirmed `Remove old app` actions.
- [x] Deduplicate/focus the migration window and preserve the existing main-app
  launch path on runtimes that still deliver `onLaunched`.
- [x] Set a stable direct uninstall URL with variant/campaign attribution.
- [x] Verify that the implementation adds no new broad permission and does not
  modify the torrent engine or user data.

### 4. Add the stable website migration route

- [x] Add `website/src/pages/migrate.astro` using the existing website layout
  and current download destinations.
- [x] Put the detected/reported platform first without hiding the other
  supported choices.
- [x] Treat extension detection as progressive enhancement and distinguish the
  extension from its required backend.
- [x] Preserve the documented `ref`, `variant`, `platform`, and `campaign`
  parameters through relevant download links without adding user identifiers.
- [x] Keep the route useful if RSTorrent later becomes the underlying engine.
- [x] Verify `/comingsoon.html` and `new.jstorrent.com` compatibility separately.

### 5. Make paid and Lite packaging reproducible

- [x] Replace or repair the stale legacy packaging scripts that reference a
  nonexistent top-level `legacy-app/` directory or mutate source files in
  place.
- [x] Represent paid/Lite names and monotonically increasing versions as
  explicit package variants rather than a loop that rewrites localization
  source and leaves backup files.
- [x] Emit two clean ZIPs with `manifest.json` at the archive root and exclude
  repository metadata, documentation, temporary files, source backups, and
  test-only identity material.
- [x] Add a validator that inspects both ZIPs, their versions/names, required
  migration files, permissions, URLs, and absence of stale waitlist and
  maximum-nag strings.
- [x] Compare each candidate's file list against its exact Store baseline and
  explain every added, removed, or changed path.

### 6. Run local automated validation

- [x] Run the focused legacy migration tests and package validator.
- [x] Run the website build and inspect the generated migration page.
- [ ] Run repository documentation and formatting checks.
- [ ] Record exact commands and results in the implementation record below.

Expected gates include:

```bash
node --test tests/legacy-packaged-app-migration.test.mjs
pnpm --dir website build
pnpm docs:check
pnpm format
git diff --check
```

Use the actual checked-in test and package-script entry points if their final
names differ; update this tactical rather than leaving aspirational commands.

### 7. Validate on the physical Chromebook

Use the public Machine Control checkout for all target readiness, staging,
desktop inspection, input, and captures. Private inventory supplies the
physical selector and credentials; do not copy them into this repository.

Start with:

```bash
cd ~/code/machine-control
bin/machine-control inventory status
bin/machine-control --target chromeos target doctor
platforms/chromeos/bin/chromeos doctor
```

Read `platforms/chromeos/skills/SKILL.md` before operating the device. Do not
repair, update, reboot, log in, or change power policy merely because a command
is available. A campaign run that explicitly requires login or reboot must use
the documented secure path and the user's authorization for that run.

Validate both paid and Lite transition fixtures, with full behavior on at
least one and a variant smoke on the other:

- [x] Stage the exact baseline and candidate as unpacked fixtures with stable
  public test identity, using `chromeos deploy-ext` and the normal
  `chrome://extensions` flow.
- [x] Seed the old waitlist-dismissed state, apply the candidate update, and
  prove no notification appears merely because `onInstalled` ran.
- [ ] Perform an explicitly authorized ordinary reboot or equivalent genuine
  profile startup, securely restore the selected user session, and prove that
  the migration notification appears near startup.
- [x] Capture the rendered title, body, buttons, icon, and persistence with
  Machine Control screenshot/accessibility commands.
- [ ] Restart again inside the seven-day interval and prove there is no second
  prompt.
- [x] Exercise reminder, acknowledgment, stable-page routing, and no-prompt
  after acknowledgment. Use controlled storage timestamps rather than waiting
  seven calendar days for the due-reminder case.
- [x] Where possible, exercise explicit legacy launch and verify the richer
  window. Record the expected current-ChromeOS absence of `onLaunched` rather
  than treating it as a regression.
- [ ] Verify that the stable page leads to honest Android, extension-plus-
  companion, and Crostini choices.
- [ ] Restore the test profile/device state and keep captures outside the
  repository; record only sanitized evidence and paths intended for the local
  handoff.

### 8. Validate on the Windows VM

Use Machine Control's common Windows target, exclusive claim, inner-first UI
routes, and cleanup policy. Start with:

```bash
cd ~/code/machine-control
bin/machine-control inventory status
bin/machine-control inventory credentials winvm
bin/machine-control --target windows target doctor
```

Then acquire an ordinary target-use claim with truthful session metadata,
record the initial power state, pass the claim to every operation, renew it if
needed, and release it in trap/finally cleanup. If this campaign starts an
inactive VM, cleanly shut it down while the claim remains held; leave an
inherited running VM running. Read
`platforms/windows/skills/drive-winvm/SKILL.md` before operating it.

Use PowerShell/SSH for staging and Machine Control's semantic Windows UI route
for Chrome. Provider screenshot or coordinate input is recovery-only and
requires the applicable disruptive claim.

- [x] Load exact baseline and candidate fixtures in a disposable Chrome
  Developer Mode profile without disturbing an unrelated browser profile.
- [x] Prove the candidate update is silent until Chrome/profile startup.
- [x] Start Chrome through the interactive desktop route and verify the
  platform-specific notification text and seven-day throttle.
- [x] Attempt body/primary activation, record whether Windows routes it back to
  the disposable Chrome profile, test the `window.open` fallback, and keep the
  short URL visible when native toast activation is not delivered.
- [x] Verify the prompt interval and acknowledgment state across complete
  Chrome process restarts. Record any toast-action delivery limitation
  separately from the campaign state behavior.
- [ ] Verify the website's Windows destination and that the extension is
  described as optional integration rather than the torrent engine.
- [x] Capture the visible result and semantic state where available, restore
  the disposable profile, and perform claim-aware VM cleanup.

Windows Developer Mode evidence proves runtime behavior, not that the Chrome
Web Store will deliver an update to a grandfathered Windows installation.
Record that distinction explicitly.

### 9. Prepare a release handoff without publishing

- [ ] Record both candidate ZIP hashes, exact baseline-to-candidate diffs,
  automated results, ChromeOS evidence, Windows evidence, and known platform
  limitations.
- [ ] Confirm the stable migration page is deployed before any Store package
  references it.
- [ ] Prepare controlled-profile Store delivery checks for both existing item
  IDs.
- [ ] Stop before upload, submission, publication, listing edits, or rollout
  unless the user explicitly authorizes those state-changing actions.
- [ ] After an authorized rollout, verify exact Store-delivered versions on a
  controlled existing installation before treating repository/device testing
  as delivery proof.

## Acceptance Criteria

- Paid and Lite candidates derive from reconciled exact Store baselines and
  have independently correct names, versions, and upload artifacts.
- The update event activates a new campaign but displays no migration UI.
- The first eligible browser startup displays one accurate native
  notification; another startup within seven days does not.
- Reminder, acknowledgment, permanent-disable, and same-campaign update state
  behave deterministically.
- No script-load prompt, automatic migration app window, automatic migration
  tab, repeating migration alarm, ten-minute migration timer, waitlist copy,
  or extension-only success claim remains.
- Notification activation and the uninstall URL use a stable owned migration
  route with bounded platform/variant/campaign attribution.
- The local migration window is useful where explicit launch still works and
  never silently removes the app.
- The migration page accurately explains desktop, Android/ChromeOS, and
  Crostini paths and can be revised for RSTorrent without another packaged-app
  update.
- Focused tests, package validation, website build, documentation checks, and
  formatting pass.
- A physical Chromebook proves the startup experience and throttle for the
  exact candidate behavior.
- A Windows VM proves the desktop notification, routing fallback, and state
  behavior while clearly separating Developer Mode evidence from Store
  delivery.
- No Store update is published without explicit authorization and a recorded
  rollback/corrective-package path.

## Implementation Record

Fill this section as work lands. Do not mark the tactical complete based only
on source changes.

| Checkpoint | Status | Evidence |
| --- | --- | --- |
| Exact paid/Lite baseline reconciliation | complete | Google update artifacts fetched and hashed on 2026-08-26; paid/Lite executable trees are byte-identical. |
| Campaign state and automated tests | complete | Ten focused Node tests cover activation, startup throttle, retry, reminder, acknowledgment, permanent stop, same-campaign preservation, explicit launch, variants, and platform routing. |
| Notification and local migration UI | complete | Shared migration runtime replaces the unshipped maximum-nag experiment; startup is the sole automatic UI trigger and explicit launch opens a bounded local window. |
| Stable website migration route | implemented; deployment pending | Astro check/build and three focused tests pass. Desktop and 390 px Playwright renders were inspected; the ChromeOS query put Play and Crostini first and preserved only bounded attribution on outgoing links. Existing `jstorrent.com/comingsoon.html` returns 200, and the intentional `new.jstorrent.com` 404 document still preserves the requested path in its browser redirect. The new live `/migrate` route remains 404 until this website commit is deployed. |
| Reproducible paid/Lite packages | complete | Deterministic builder and validator emit 122-file ZIPs plus public-key-only unpacked test fixtures. Paid `2.4.5`: `e76b7440928c1b6429775a2004c66d1f485cc07457efd976b4f6f129c1a18ac7`; Lite `2.4.13`: `0821158b3f90776ce061e187994cf8c82b476cb3af05dddadff299c82953cd91`. |
| Physical ChromeOS validation | in progress; startup-event proof pending | The exact paid baseline `2.4.4` updated silently to `2.4.5` under the paid Store ID, preserved the old waitlist flags, and armed the new campaign with `lastPromptedAt: 0`. The paid notification and actions and the Lite `2.4.13` identity/copy were accepted in the real ChromeOS notification center. A genuine profile startup and second-startup throttle check still require the secure post-restart login path. |
| Windows VM validation | complete with toast-activation limitation | Chrome 151 in an isolated Developer Mode profile loaded paid `2.4.4` under the real Store ID, updated silently to `2.4.5`, and produced the desktop-specific native toast only after a complete clean Chrome restart. A second restart inside seven days produced no toast. Windows rendered the body and buttons but did not route body or action activation to the Chrome App; the visible short URL is the required fallback. Direct handler acceptance proved `window.open`, acknowledgment, local UI, and native removal. |
| Controlled Store delivery | pending | Requires separate authorization. |

Deliberate deferrals, rejected claims, and platform-specific limitations must
be recorded here during execution so the completed tactical describes what
was actually proven.

### Candidate artifact reconciliation

The 2026-08-26 paid and Lite upload candidates are byte-reproducible across
two consecutive local builds. The validator also checks their contents rather
than trusting filenames or build output. Each candidate differs from its exact
Store baseline by the same reviewed path set:

- removed: `LICENSE`, `README.md`, `TODO.txt`, the accidentally shipped
  `_locales/en/messages.json.bak`, and Store-generated
  `_metadata/verified_contents.json`;
- added: `migrate.html`, `migrate.js`, `migration.js`, and
  `migration-runtime.js`; and
- changed: `background.js` and `manifest.json` only.

The localized messages file remains byte-identical to each matching Store
baseline. The candidate manifest changes only the monotonically increasing
variant version and the background script list. The background change points
uninstall attribution to `/migrate`, suppresses the obsolete immediate generic
update toast, and removes the old waitlist campaign; the new bounded behavior
lives in the four added migration files.

Shipping ZIPs contain no manifest `key`. Ignored unpacked fixtures contain
only the public Store keys, which the builder verifies derive
`anhdpjpojoipgpmfanmedjghaligalgb` and
`abmohcnlldaiaodkpacnldcdnjjgldfh`. No private key is present or required.

### Physical ChromeOS evidence

The 2026-08-26 physical-device run used the public Machine Control provider
and a normal Developer Mode unpacked-app flow. It established the following
without changing the Store upload artifacts:

- The paid baseline fixture loaded as ID
  `anhdpjpojoipgpmfanmedjghaligalgb`, version `2.4.4`. After seeding the old
  `migrationNoticeDismissed` state and applying the candidate, Chrome reported
  an update from `2.4.4` to `2.4.5`. No notification appeared during that
  update. The new `available-2026` state was active with
  `lastPromptedAt: 0`, proving that the old waitlist dismissal neither
  suppressed nor prematurely displayed the new campaign.
- The unpacked-update harness briefly retained one initialization exception
  after copying new files over the running baseline. Inspection proved that
  the stable candidate's generated background page loaded `conf.js`,
  `migration.js`, `migration-runtime.js`, and `background.js` in the declared
  order. A clean candidate reload reproduced no exception, and the app card
  had no remaining error. This was a non-atomic unpacked-directory harness
  artifact, not an upload-ZIP ordering defect.
- The paid native notification rendered with the JSTorrent icon, the title
  `JSTorrent has a new version`, the short owned URL in its ChromeOS-specific
  body, and `See migration options` / `Remind me in 7 days` actions. Chrome's
  notification API retained it and the system notification center exposed the
  complete card through accessibility.
- Activating `Remind me in 7 days` cleared the card and wrote a snooze exactly
  seven days in the future. Re-running the startup handler while snoozed
  created no notification. Moving only the controlled campaign timestamp past
  the interval created one new card.
- Activating the primary action cleared the card, recorded acknowledgment, and
  opened exactly
  `https://jstorrent.com/migrate?ref=legacy-app-notification&variant=paid&campaign=available-2026&platform=chromeos`.
  The route currently returns the expected GitHub Pages 404 because website
  deployment is still a release gate; notification routing itself is proven.
- The local migration window rendered accurate ChromeOS composition guidance,
  correctly detected the installed new extension without claiming setup was
  complete, and its `Stop reminders` action persisted `disabledAt` and closed
  the window.
- Launching the app from the real ChromeOS launcher did not deliver
  `chrome.app.runtime.onLaunched`. Chrome displayed its system-owned
  `JSTorrent can't be launched` dialog explaining that Chrome Apps stopped
  running on ChromeOS in July 2025. This confirms that startup notification is
  the viable automatic channel on current ChromeOS; the local window remains
  useful only on runtimes or controlled calls that can still create it.
- The Lite candidate loaded independently as ID
  `abmohcnlldaiaodkpacnldcdnjjgldfh`, name `JSTorrent Lite`, version `2.4.13`.
  Its fresh campaign was active and unprompted, and its real native card used
  the variant-specific title `JSTorrent Lite has a new version` with the same
  bounded migration actions and ChromeOS copy.

Representative captures remain outside the repository at:

- `/tmp/jstorrent-chromeos-update-devtools-state.png`
- `/tmp/jstorrent-chromeos-paid-notification-center.png`
- `/tmp/jstorrent-chromeos-paid-migration-window.png`
- `/tmp/jstorrent-chromeos-after-real-launch-click.png`
- `/tmp/jstorrent-chromeos-lite-notification-center.png`

The remaining ChromeOS acceptance gap is event provenance, not notification
rendering or state behavior: perform a genuine profile startup, observe the
first due notification without directly invoking its handler, then perform a
second startup inside seven days and observe no new card. Secure login input
is not present in the machine inventory, so that step must use the documented
human/approved-secret channel rather than placing a PIN in commands or logs.

### Windows VM evidence

The 2026-08-26 Windows run used an exclusive ordinary-use Machine Control
claim and a disposable Chrome profile on Windows 11. The VM was stopped before
the run and was cleanly returned to that state before the claim was released.
The run established:

- Chrome `151.0.7922.174` still loads unpacked Chrome Apps in Developer Mode.
  The exact paid fixture appeared as ID
  `anhdpjpojoipgpmfanmedjghaligalgb`, version `2.4.4`.
- After the old waitlist state was seeded, the fixture directory was replaced
  with the exact 122-file paid candidate and reloaded. The card reported
  `2.4.5`; no toast or other migration UI appeared. The stable generated
  background page loaded the four declared scripts in order, retained the old
  dismissal fields, and armed `available-2026` with `lastPromptedAt: 0` and no
  notification.
- A complete clean Chrome exit and interactive relaunch fired the real startup
  path. Windows displayed a native Google Chrome toast with the JSTorrent icon,
  `JSTorrent has a new version`, the desktop copy, visible
  `jstorrent.com/migrate`, and both migration/reminder buttons. UI Automation
  exposed the complete card and button names.
- Clearing that card without changing campaign state, then performing another
  complete clean Chrome exit/relaunch inside seven days, produced no toast and
  no Chrome notification. The persisted `lastPromptedAt` value was unchanged.
- Windows did not deliver native toast activation to the disposable Chrome App
  profile. UI Automation tested InvokePattern and a focused physical click on
  `Remind me in 7 days`, a focused physical click on `See migration options`,
  and a focused click on the notification body. Each dismissed the visible
  toast but created no tab and triggered no state transition; Chrome continued
  to report the notification internally. This matches the earlier 200 OK
  Chrome-App migration evidence and makes the visible short URL essential.
- The app-side notification handler remains correct if Chrome delivers the
  callback: `chrome.browser` is undefined on this Windows runtime, so the real
  fallback used `window.open` to open exactly
  `https://jstorrent.com/migrate?ref=legacy-app-notification&variant=paid&campaign=available-2026&platform=windows`
  and persisted acknowledgment. Moving the prompt time eight days into the
  past and restarting Chrome still produced no toast after acknowledgment.
- The bounded local window rendered honest desktop guidance: install the
  standalone desktop app, with the extension described as optional browser
  integration. Its `Remove old app` action displayed Chrome's native
  `Remove "JSTorrent"?` confirmation with abuse-report, Remove, and Cancel
  controls. Confirming Remove removed only the disposable legacy app.
- The live migration URL still returned the documented deployment-gate 404.
  Therefore the exact route and platform attribution are accepted, but the
  Windows page destination cannot be called live-accepted until the website is
  deployed.

Representative captures remain outside the repository at:

- `/tmp/jstorrent-windows-paid-baseline.png`
- `/tmp/jstorrent-windows-paid-update-silent.png`
- `/tmp/jstorrent-windows-paid-startup-notification.png`
- `/tmp/winvm-ui.nIpjEE/screenshot.png` (local migration window)
- `/tmp/winvm-ui.tZqi2C/screenshot.png` (native removal confirmation)

Cleanup deleted the exact disposable Windows staging/profile directory after
the legacy app was removed and Chrome exited. Those generated test files are
not recoverable and contained no user data. The target doctor passed before a
clean guest shutdown; final target status was powered off and the exclusive
claim was released.
