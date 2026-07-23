# Legacy Chrome App Migration

How we migrate users from legacy JSTorrent apps to the new Chrome Extension.

> **Cross-reference:** Web Server for Chrome has a parallel migration effort with the same architecture.
> See `~/code/web-server/docs/legacy-migration.md`. Changes here should be reflected there and vice versa.

## Apps (4 total)

| App | Type | ID | Users | Status |
|---|---|---|---|---|
| JSTorrent | Chrome App | `anhdpjpojoipgpmfanmedjghaligalgb` | ~60k | Dead on Chrome 144, needs migration |
| JSTorrent Lite | Chrome App | `abmohcnlldaiaodkpacnldcdnjjgldfh` | ? | Dead CWS listing, users still have it installed |
| JSTorrent Helper Extension | Extension (MV3 in repo, **verify what's live on CWS**) | `bnceafpojmnimbnhamaeedgomdcgnbjk` | ~10k | Alive. Context menu "Add to JSTorrent", sends messages to legacy Chrome Apps |
| JSTorrent (new) | Extension (MV3) | `dbokmlpefliilbjldladbimlcfgbolhk` | new | The replacement for all of the above |

**To verify:**
- Is the published helper extension on CWS actually MV2 or MV3? The copy in `archive/legacy-extension/` is MV3 (`service_worker`) but we're not sure if that version was ever pushed to CWS.
- How many users does JSTorrent Lite still have installed?

### Helper Extension

The helper extension (`archive/legacy-extension/`) adds a right-click "Add to JSTorrent" context menu for magnet links and .torrent files. It sends messages to the legacy Chrome App IDs via `chrome.runtime.sendMessage`. It has ~10k users — a significant migration channel itself.

Its `externally_connectable` lists both legacy Chrome App IDs:
```json
"ids": ["anhdpjpojoipgpmfanmedjghaligalgb", "abmohcnlldaiaodkpacnldcdnjjgldfh"]
```

**Migration path for helper extension: TBD.** Options include:
- Update it to detect and message the new extension instead of / in addition to the legacy apps
- Merge its context menu functionality into the new extension and push a final update directing users there
- Use it as a migration channel: push an update that notifies its 10k users about the new extension

**Bug (fixed):** ~~`EXTENSION_CWS_URL` pointed to the helper extension~~ → Now `NEW_EXTENSION_CWS_URL` points to the new extension (`dbokmlpefliilbjldladbimlcfgbolhk`).

## Chrome 144 Impact (Jan 2026)

Chrome 144 completely blocks Chrome App launches at the OS level on ChromeOS.

**What still works:**
- `chrome.runtime.onStartup` — fires on every browser boot. Background page loads, all code executes. **Primary migration channel.**
- `chrome.runtime.onInstalled` — fires on CWS update push
- `chrome.runtime.onMessageExternal` — websites can still message the legacy app (wakes background page)
- `chrome.browser.openTab` — can force-open browser tabs
- `chrome.management.uninstallSelf` — shows confirmation dialog, lets app remove itself
- `chrome.notifications` — notifications still display, buttons work
- Uninstall URL redirect — catches users who manually remove the app

**What's broken:**
- `chrome.app.runtime.onLaunched` — **never fires**. Launcher blocked at OS level. User clicks icon → one-time dialog: "Chrome apps stopped running on ChromeOS devices in July 2025." Subsequent clicks silently do nothing.

## Migration Nag System

### Current State (`archive/legacy-app/background.js`)

Now matches Web Server for Chrome's aggressive pattern. All behavior is controlled by config flags at the top of the migration section, making it easy to dial aggressiveness up/down across CWS pushes.

```javascript
var NEW_EXTENSION_ID = 'dbokmlpefliilbjldladbimlcfgbolhk'
var NEW_EXTENSION_CWS_URL = 'https://chromewebstore.google.com/detail/jstorrent/dbokmlpefliilbjldladbimlcfgbolhk'

var MIGRATE_ON_SCRIPT_LOAD = true   // nag every time the event page loads (any event)
var MIGRATE_ON_STARTUP = true       // nag on chrome.runtime.onStartup (browser boot)
var MIGRATE_ON_INSTALLED = true     // nag on chrome.runtime.onInstalled (CWS update push)
var MIGRATE_ON_LAUNCHED = true      // nag on chrome.app.runtime.onLaunched — dead on Chrome 144+
var MIGRATE_USE_ALARM = true        // set repeating alarm to nag periodically
var MIGRATE_ALARM_MINUTES = 10      // alarm interval in minutes
```

### Triggers

- **Script load** (catch-all) — fires on every event page wake, regardless of which event caused it
- **onStartup** — browser boot
- **onInstalled** — CWS update push; also sets up repeating alarm + uninstall URL
- **onLaunched** — dead on Chrome 144+ but kept for users on older Chrome
- **Alarm** — repeating, fires every `MIGRATE_ALARM_MINUTES`

All triggers call `showMigrationNags(reason)` which fires both:
1. **Notification** — `requireInteraction: true`, platform-aware messaging (ChromeOS vs desktop), `[reason]` tag for debugging
2. **Migrate window** — `chrome.app.window.create('migrate.html')`, ChromeOS only

### Migrate Window (`archive/legacy-app/migrate.html`)

Standalone app window that:
1. Pings the new extension via `chrome.runtime.sendMessage(NEW_EXTENSION_ID, {type: 'ping'})`
2. If not installed: shows "Get the new extension" CWS link + "Remind me later"
3. If installed: shows "You're all set!" + "Remove old app" (`chrome.management.uninstallSelf`) + "Keep for now"

### No persistent dismiss

Unlike the old approach, there is no `migrationNoticeDismissed` flag. Every trigger fires the nag. This is intentional — the app icon is dead and users need to migrate.

### Comparison with Web Server

| | Web Server | JSTorrent |
|---|---|---|
| Config flags | Yes (all on) | Yes (all on) |
| Script load nag | Yes | Yes |
| Repeating alarm | Every 10 min | Every 10 min |
| Migrate window | Yes (chrome.app.window) | Yes (chrome.app.window) |
| Dismiss behavior | No dismiss | No dismiss |
| Feature flag | None | None |
| Platform detection | navigator.userAgent | chrome.runtime.getPlatformInfo |
| New ext detection | migrate.html pings new ext | migrate.html pings new ext |

## External Messaging (`externally_connectable`)

### Message Flow (current)

```
jstorrent.com ──sendMessage──► Legacy Chrome Apps (full + lite)
Helper Extension ──sendMessage──► Legacy Chrome Apps (full + lite)
jstorrent.com ──sendMessage──► New Extension
```

The new extension's `externally_connectable` now lists all legacy IDs, enabling bidirectional detection (like Web Server). The legacy app's `migrate.html` pings the new extension to check if it's installed.

### Legacy Helper Extension (`archive/legacy-extension/manifest.json`)

```json
"externally_connectable": {
    "ids": [
        "anhdpjpojoipgpmfanmedjghaligalgb",
        "abmohcnlldaiaodkpacnldcdnjjgldfh"
    ]
}
```

Accepts messages from both legacy Chrome Apps. Sends context menu actions (magnet links, .torrent URLs) to whichever legacy app is installed.

### New Extension Manifest (`extension/public/manifest.json`)

```json
"externally_connectable": {
    "ids": [
        "anhdpjpojoipgpmfanmedjghaligalgb",
        "abmohcnlldaiaodkpacnldcdnjjgldfh",
        "bnceafpojmnimbnhamaeedgomdcgnbjk"
    ],
    "matches": [
        "https://new.jstorrent.com/*",
        "https://jstorrent.com/*",
        "http://local.jstorrent.com/*"
    ]
}
```

Accepts messages from jstorrent.com websites, both legacy Chrome App IDs, and the helper extension. The service worker handles `{type: 'ping'}` and responds with `{ok: true, installed: true}`.

## Website Traffic (primary migration touchpoints)

From Cloudflare analytics (Feb 28, 2026, last 24h):

| Page | Hits/day | New ext detection | Migration banner | Notes |
|---|---|---|---|---|
| `/add/` | ~148 | Yes (tries first) | Yes | Magnet handler. **Done.** |
| `/` | ~75 | N/A | N/A | Main site |
| `/comingsoon.html` | ~40 | N/A | N/A | Migration landing page |
| `/launch` | ~25 | Yes (new ext only) | No | New launcher page |
| `/stream/` | ~15 | No (manual param) | No | Legacy streaming player |
| `/share/` | small | No | No | Legacy share page, needs update |

**By OS:** ChromeOS dominates (~236 of ~314 total, ~75%).

### `/add/` Page (done)

`jstorrent.com/add/#magnet_uri=magnet:?xt=...`

1. Pings new extension first (`{type: 'ping'}` → `{type: 'launch-ping', magnet}`)
2. Falls back to legacy app IDs (`{command: 'add-url'}`)
3. If legacy detected but new extension not: shows migration banner with CWS link
4. If nothing detected: shows install prompt

### `/share/` Page (needs update)

Still only messages legacy app IDs. No new extension detection, no migration banner. Should either be updated with the same detection logic as `/add/` or redirected to `/add/`.

### `/stream/` Page (low priority)

Takes app ID from URL hash parameter. No automatic detection. Low traffic (~15/day).

## Migration Channels (ranked by reach)

1. **Legacy Chrome App CWS update push** (~60k) — `onStartup` fires on every boot + `chrome.browser.openTab` can force-open jstorrent.com. Reaches all installed users. Need to confirm CWS still accepts updates.
2. **`/add/` page update** (~285 hits/day) — Active magnet-clicking users. Can detect new extension and route magnets to it. Fix broken install button. Immediate, no CWS approval needed.
3. **Helper extension CWS update push** (~10k) — Push an update that notifies users about the new extension. These are active extension users, high conversion potential.
4. **`/share/` page update** — Similar external messaging path, add migration banner + new extension detection.
5. **Legacy CWS listing updates** — Update descriptions to say apps have been replaced. Users who can't launch will visit the CWS page.
6. **Buttondown email blast** — 863 subscribers
7. **Social media** — X, Reddit, Discord, LinkedIn

## Deploy & Test Scripts

| Script | Purpose |
|---|---|
| `scripts/deploy-legacy-app.sh` | rsync `archive/legacy-app/` to Chromebook via SSH for unpacked testing |
| `scripts/deploy-chromebook.sh` | Deploy extension to Chromebook |

### Testing on Chromebook

```bash
# Deploy legacy app
./scripts/deploy-legacy-app.sh
# Load as unpacked at chrome://extensions (Developer mode)
```

## Analytics Ref Parameters

- `?ref=app-notification` — User interacted with migration notification
- `?ref=legacy-auto` — Tab opened automatically on boot (planned)
- `?ref=uninstall` — Uninstall URL redirect

## Open Questions

### To verify
- What manifest version is the helper extension on CWS? The repo copy (`archive/legacy-extension/`) is MV3, but unclear if that was ever published.
- How many users does JSTorrent Lite still have?
- Can we still push updates to the legacy Chrome Apps on CWS?
- Are Chrome App CWS updates still being delivered to existing installs?

### To decide
- Helper extension migration path: update to route to new extension, merge into new extension, or both?
- Update `/share/` page: add new extension detection or redirect to `/add/`?
- What alarm interval to ship with? (Currently 10 min — very aggressive, good for testing, may want to increase for production)
- What flags to ship with initially? (Start conservative, increase aggressiveness over successive CWS pushes)

### Done
- ~~`EXTENSION_CWS_URL` bug~~ → Fixed: `NEW_EXTENSION_CWS_URL` points to new extension
- ~~Add legacy IDs to new extension's `externally_connectable`~~ → Done
- ~~Aggressive nag pattern~~ → Implemented (matches Web Server)
- ~~`/add/` page new extension detection~~ → Done

## Files

| File | Purpose |
|---|---|
| `archive/legacy-app/background.js` | Migration nag system (config flags + triggers) |
| `archive/legacy-app/migrate.html` | Migration app window (pings new extension, CWS link, uninstall button) |
| `archive/legacy-extension/manifest.json` | Legacy helper extension manifest |
| `extension/public/manifest.json` | New extension manifest (externally_connectable includes legacy IDs) |
| `extension/src/sw.ts` | New extension service worker (handles ping from migrate.html) |
| `website/public/add/index.js` | `/add/` page — magnet handler, #1 traffic page (migration done) |
| `website/public/share/index.js` | `/share/` page — legacy, needs update |
| `website/public/comingsoon.html` | Migration landing page |
| `scripts/deploy-legacy-app.sh` | Deploy legacy app to Chromebook |
| `docs/project/CHROMEOS-STRATEGY.md` | ChromeOS architecture (extension + Android IO daemon) |
