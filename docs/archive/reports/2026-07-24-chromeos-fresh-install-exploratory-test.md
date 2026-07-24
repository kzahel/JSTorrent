# ChromeOS Fresh-Install Exploratory Test

Date: 2026-07-24

Result: Pass, with automatable friction points

## Outcome

A physical Chromebook completed the intended first-run journey:

`unpacked extension -> Play Store install -> launch chooser -> pairing -> SAF
root consent -> Ready -> controlled torrent download`

The downloaded 100 MiB fixture reached 100%/seeding and independently matched
the source SHA-256:

```text
9a0069c0949fca3d18c0c3371e931ad1af23d36c9a04e9c116490d9694b4adfd
```

The sampled Android log contained no fatal or exception entries. The testbed
passed 10 post-run doctor checks with no failures.

## Environment

| Component | Value |
| --- | --- |
| Repository revision | `34fac24eabe43758bd882a9a779a9cedce902a63` |
| ChromeOS | Official build 16700.46.0, milestone 150 |
| Board | `nami-signed-mp-v12keys` |
| Extension | unpacked MV3 1.1.1, `dbokmlpefliilbjldladbimlcfgbolhk` |
| Android app | Play Store 1.0.23, versionCode 23 |
| Android installer | `com.android.vending` |
| Companion endpoint | `100.115.92.2:7800` |
| Test torrent | deterministic `testdata_100mb.bin` |
| Info hash | `67d01ece1b99c49c257baada0f760b770a7530b9` |

The pre-existing MV3 extension and Android package were removed. The unrelated
legacy Chrome App and unrelated extensions were preserved. Removing the
Android package did not remove previously downloaded shared-storage content.

## Observed Flow

| Stage | Result | Evidence |
| --- | --- | --- |
| Testbed baseline | Pass | Doctor healthy; signed-in desktop reachable |
| ARCVM ADB | Pass after visible authorization | ChromeOS showed the expected USB-debugging prompt |
| Extension build/deploy | Pass | Build copied directly to `Downloads/jstorrent-extension`; Crostini was unnecessary |
| Load unpacked | Pass | Extension 1.1.1 loaded with stable ID and no errors |
| App-absent state | Pass | UI showed Offline, then Setup Required |
| Play Store handoff | Pass with extra foreground step | Link opened the web listing; Chrome's open-in-app affordance launched the native listing but left Chrome in front |
| Native install | Pass | Play Store installed 1.0.23; ADB confirmed `com.android.vending` |
| Companion launch | Pass with chooser | **Launch App** produced an **Open with** confirmation for JSTorrent |
| Pairing | Pass | Android displayed **Allow Connection?**; extension changed to Connected after Allow |
| Download root | Pass with two confirmations | SAF preselected `Download/JSTorrent`, then required **Use this folder** and **Allow** |
| Engine readiness | Pass | Extension reached Ready with the selected JSTorrent root |
| Controlled transfer | Pass | One peer, 100%, seeding; source/device SHA-256 values matched |
| Post-run health | Pass | Doctor: 10 passed, 0 failed; sampled logcat had no fatal/exception matches |

The transfer started at 14:53:50 local time and the completed screenshot was
captured at 14:55:01. The initial UI estimate was slow, but the 100 MiB fixture
completed in about 71 seconds.

## Friction and Automation Findings

1. **The Play Store path is two-stage.** **Get from Play Store** first opens
   `play.google.com`; the Chrome open-in-app affordance is needed to reach the
   native listing. The native Play Store window may open behind Chrome, so the
   harness must wait for and focus the `Play Store` application.
2. **Launch has another chooser.** The extension's **Launch App** intent opens
   a Chrome **Open with** bubble. This is a real first-run confirmation unless
   the user remembers the choice.
3. **Shared storage is not fresh after package uninstall.** The existing
   `Download/JSTorrent` root still contained earlier public-domain downloads,
   including a complete Big Buck Bunny. A test must use a unique root/fixture
   or narrowly clean only its own files.
4. **Storage setup is part of onboarding.** Pairing alone leaves the extension
   in Setup until the user completes two SAF confirmations.
5. **Accessibility coverage is mixed.** Browser UI was reliably semantic.
   ChromeOS dialogs, Play Store, Android Compose, and DocumentsUI sometimes
   exposed only static text or no actionable node; OCR and the absolute pointer
   remain required fallbacks.
6. **Removing the old extension navigates away.** Chrome opened the uninstall
   feedback page after confirmation, so an automated run must explicitly
   return to `chrome://extensions`.
7. **Toolbar Remove retains data.** Removing the controlled torrent cleared
   the session row immediately but left `testdata_100mb.bin`; the run deleted
   only that known fixture through ADB.
8. **The local package-manager path drifted.** The shell resolved pnpm 9.15.1,
   while the repository requires pnpm >=11 and declares 11.16.0. The build
   succeeded with `npx --yes pnpm@11.16.0 --filter jstorrent-extension build`.
   Automation should use the repository-declared toolchain deterministically.

## Recommended Automated Layers

### Per-change physical APK smoke

- Run `chromeos doctor` and `chromeos smoke-test`.
- Build/deploy the unpacked extension and current debug APK.
- Start from cleared private app/extension state while preserving unrelated
  user data.
- Drive launch chooser, pairing, SAF consent, and Ready.
- Seed the deterministic 100 MiB fixture.
- Route peer traffic through a temporary SSH reverse plus ADB reverse rather
  than changing the workstation firewall.
- Require 100% and compare SHA-256 values.
- Capture screenshots, package metadata, logcat errors, doctor output, and a
  manifest.

### Scheduled Play Store acceptance

- Uninstall only `com.jstorrent.app`.
- Exercise **Get from Play Store** through the web and native listings.
- Install and assert `installerPackageName=com.android.vending`.
- Reuse the remaining pairing, root, and transfer assertions.
- Keep this lane manual or scheduled because it depends on a signed-in Play
  account and mutable store state.

### APK-change acceptance

Use `./scripts/deploy-android-chromebook.sh` for the current debug or release
APK. For a truly fresh APK test, uninstall `com.jstorrent.app` first; the normal
install command is an update-oriented developer loop. The same downstream
onboarding scenario should then verify that a new APK still pairs, grants a
root, transfers data, and survives independent hashing.

## Artifacts and Cleanup

The run artifacts were collected locally under:

```text
/tmp/jstorrent-chromeos-fresh-install-20260724TM1wXOB
```

That directory contains 41 numbered screenshots, the testbed smoke artifacts,
and `post-run-diagnostics/manifest.json`. High-signal frames include:

- `08-extension-first-launch-app-absent.png`
- `12-native-play-store.png`
- `16-launch-intent.png`
- `18-pairing-state.png`
- `20-post-pairing.png`
- `22-folder-picker.png`
- `24-folder-permission.png`
- `26-ready-after-folder.png`
- `30-file-selection-ready.png`
- `31-download-started.png`
- `32-download-progress.png`

Cleanup removed only `testdata_100mb.bin`, its torrent session row, the ADB
reverse, the SSH reverse, and the local seeder process. The Play Store app,
pairing, selected root, unpacked extension, legacy Chrome App, and pre-existing
shared downloads were left intact for follow-up testing.
