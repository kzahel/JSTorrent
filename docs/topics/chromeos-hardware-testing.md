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

## Optional Crostini

Crostini is not part of normal extension or APK deployment. Start it only for
Linux-container work or ChromeOS folders explicitly shared with Linux:

```bash
"$TESTBED" crostini-status
"$TESTBED" crostini-start
ssh chromebook
"$TESTBED" crostini-stop
```

`crostini-start` launches `termina`/`penguin`, restores the post-reboot
port-forwarding rule, and requires authenticated `ssh chromebook` before
returning success.

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
