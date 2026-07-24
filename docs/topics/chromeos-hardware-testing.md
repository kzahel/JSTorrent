# ChromeOS Hardware Testing

Topic: chromeos-hardware-testing

Status: active and functional

Last reconciled: 2026-07-24

## Authority

The physical-device controller is the separate
[`kzahel/chromeos-testbed`](https://github.com/kzahel/chromeos-testbed)
repository, normally cloned at `~/code/chromeos-testbed`.

Its `skills/SKILL.md` is the operational reference for screenshots, precise
pointer and OCR control, ChromeOS accessibility, Chrome DevTools, ARCVM ADB,
Crostini, diagnostics, reboot recovery, and closed-lid operation. JSTorrent
owns only its project-specific build and deployment entry points. Do not copy
the testbed implementation into this repository.

Documents under `docs/archive/` describe earlier workflows and are not current
authority. In particular, current extension and APK deployment do not require
Crostini.

## Host Roles

| Alias | Role | Use |
| --- | --- | --- |
| `chromeroot` | ChromeOS host, root SSH on port 2223 | Testbed CLI, UI, screenshots, DevTools, extension files, ARCVM ADB |
| `chromebook` | Optional `penguin` Crostini container on port 2222 | Linux shell, `scp`/`rsync`, and folders shared with Linux |

Start every hardware session from the JSTorrent development machine:

```bash
TESTBED=~/code/chromeos-testbed/bin/chromeos
"$TESTBED" doctor
```

`doctor` verifies whether SSH was genuinely restored by the current boot's
network-ready trigger. It does not mistake a manual VT2 start for verified
reboot persistence.

## Extension Workflow

Build, deploy directly into ChromeOS Downloads, and reload the unpacked
extension when DevTools is available:

```bash
./scripts/deploy-chromebook.sh
```

The script delegates device work to:

```bash
~/code/chromeos-testbed/bin/chromeos deploy-ext \
  extension/dist \
  --name jstorrent-extension \
  --reload dbokmlpefliilbjldladbimlcfgbolhk
```

The first load remains manual: enable developer mode in
`chrome://extensions`, choose **Load unpacked**, and select
`Downloads/jstorrent-extension`. Subsequent deployments can reload the
installed development extension through CDP.

For visual or behavioral assertions, use semantic desktop/browser operations
before raw coordinates:

```bash
"$TESTBED" screenshot
"$TESTBED" targets
"$TESTBED" desktop-tree --depth 3
"$TESTBED" screen-find-text 'JSTorrent|Install|Open'
```

## Android Companion Workflow

Build and install the ChromeOS companion APK through ARCVM ADB:

```bash
./scripts/deploy-android-chromebook.sh
```

The script delegates installation to `chromeos install-apk --authorize`.
ChromeOS may show a USB-debugging authorization prompt after a reboot or ARCVM
restart; the testbed can approve the visible prompt, but the prompt must
actually be present.

Use `--forward` when the Android app must reach a development server on this
machine:

```bash
./scripts/deploy-android-chromebook.sh --forward
```

## Fresh-Install Acceptance Flow

The physical acceptance test should cover the user-visible boundary that the
emulator companion smoke test deliberately skips:

1. Start signed in with the MV3 extension and `com.jstorrent.app` absent.
2. Deploy and load `extension/dist` as an unpacked extension.
3. Open JSTorrent, expose **Setup Required**, and follow **Get from Play
   Store** into the native Play Store listing.
4. Install the APK, then verify through ADB that the installer is
   `com.android.vending`.
5. Return to the extension, choose **Launch App**, accept Chrome's app chooser,
   and approve the Android pairing request.
6. Add a download root through the Storage Access Framework, including both
   **Use this folder** and **Allow**.
7. Require the extension to reach **Ready**, download the deterministic 100 MiB
   fixture from `seed_for_test.py`, and compare the source and device SHA-256
   values.

The Play Store leg should remain a separate, lower-frequency acceptance test.
The faster APK-development variant can replace steps 3-4 with a fresh
`./scripts/deploy-android-chromebook.sh` install while retaining the chooser,
pairing, SAF, transfer, and hash assertions.

Uninstalling the APK clears private app state but not
`Download/JSTorrent`. A deterministic fresh-install runner must use a unique
test root or remove only its own named fixture. Do not recursively clear the
shared download directory. Likewise, toolbar **Remove** currently removes the
torrent from the session but leaves its downloaded file in place.

The 2026-07-24 exploratory physical run passed this flow with extension 1.1.1,
Play Store app 1.0.23, and a hash-verified 100 MiB transfer. It also established
that the web-to-native Play Store handoff can leave Chrome in front, the launch
intent produces an **Open with** confirmation, and Play Store/Android surfaces
sometimes require desktop accessibility or absolute-pointer fallbacks. See
[`../archive/reports/2026-07-24-chromeos-fresh-install-exploratory-test.md`](../archive/reports/2026-07-24-chromeos-fresh-install-exploratory-test.md).

## Optional Crostini

Crostini is not part of normal extension or APK deployment. Start it only for
Linux-container work or ChromeOS folders explicitly shared with Linux:

```bash
"$TESTBED" crostini-status
"$TESTBED" crostini-start
ssh chromebook
"$TESTBED" crostini-stop
```

`crostini-start` launches the user-facing Terminal app, selects its `penguin`
profile, requires ChromeOS to register `penguin.linux.test`, restores the
post-reboot port-forwarding rule, and verifies authenticated `ssh chromebook`.

The extension's no-Play-Store product route additionally requires the
published installer. Run the exact user-facing command from that Terminal:

```bash
curl -fsSL https://jstorrent.com/install-crostini.sh | bash
```

The testbed deliberately does not start the container through `vmc` or LXD,
because that lower-level route can leave `cros-garcon` without a security
token, hostname resolution, or localhost tunneling. For a product assertion,
still require both of these checks before opening the extension:

```bash
curl -fsS http://localhost:7800/health
curl -fsS http://penguin.linux.test:7800/health
```

The 2026-07-24 physical run started with no installed daemon or pairing state,
ran the exact public installer, force-stopped Android, cleared extension
companion state, reset daemon pairing, and reopened the extension. Automatic
discovery and pairing reached **Ready** at `penguin.linux.test:7800`; the panel
reported **Crostini Daemon** and the fixed `Downloads` root. Installer syntax,
published asset availability, fallback checksums, and local integrity tests
also passed. A torrent payload transfer and actual ChromeOS Flex hardware
remain separate coverage gaps. See
[`../archive/reports/2026-07-24-chromeos-crostini-no-play-store-test.md`](../archive/reports/2026-07-24-chromeos-crostini-no-play-store-test.md).

## Reboots and Updates

Ordinary open- or closed-lid reboots are unattended on the configured Lenovo
testbed. Root SSH starts after ChromeOS emits `shill-connected`; the machine
can remain closed and still provide screenshots, OCR, accessibility, and the
absolute pointer.

ChromeOS updates can replace the rootfs boot job and DevTools configuration.
If `ssh chromeroot` does not return, recover once from VT2:

```bash
sudo -i
bash /mnt/stateful_partition/etc/ssh/start_sshd.sh
```

Then run `chromeos doctor`, re-run the current bootstrap if requested, and use
`chromeos fix-devtools` if the update reset remote debugging.

## Validation

For infrastructure-only checks:

```bash
"$TESTBED" doctor
```

For a restoring end-to-end UI exercise with screenshots and machine-readable
evidence:

```bash
"$TESTBED" smoke-test
```

Use JSTorrent-specific assertions after the infrastructure passes. Record the
testbed command, target build, and artifact path in the relevant topic or
change report.
