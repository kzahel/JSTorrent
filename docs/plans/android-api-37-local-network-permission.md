# Android API 37 Local Network Permission Plan

- **Status:** Deferred until JSTorrent begins targeting Android 17 / API 37
- **Recorded:** 2026-07-23
- **Current Android target:** API 36

## Decision

Do not add or request `android.permission.ACCESS_LOCAL_NETWORK` while JSTorrent
targets API 36.

Android 16 provides an opt-in compatibility mode for testing local-network
restrictions, but apps targeting API 36 or lower continue to receive implicit
local-network access through `android.permission.INTERNET`. Android's current
guidance explicitly says not to add or request `ACCESS_LOCAL_NETWORK` before
targeting API 37.

The permission work must land before, or in the same release as, the future
`targetSdk` change from 36 to 37.

## When to Revisit

Start this work when the first of these occurs:

1. JSTorrent begins its Android 17 / API 37 target SDK update.
2. Google Play announces an API 37 target deadline that applies to JSTorrent.
3. Android changes the API 36 compatibility behavior or publishes materially
   different migration guidance.

Re-read the official Android documentation at that time. Android 17 guidance
may change before the target SDK update is required.

## What Android 17 Changes

For apps targeting API 37 or higher, Android blocks local-area-network traffic
by default. Apps that need direct LAN access must either use an applicable
system-mediated picker or:

1. Declare `android.permission.ACCESS_LOCAL_NETWORK` in the manifest.
2. Check the permission before beginning LAN-dependent work.
3. Request it at runtime from an activity.
4. Handle denial and later revocation without crashing or repeatedly prompting.

The permission applies to traffic involving local-network addresses, including
outgoing and incoming TCP, UDP unicast, broadcast, multicast, and APIs layered
on top of those sockets. It does not replace the existing `INTERNET` permission
for ordinary internet traffic.

## JSTorrent Impact Audit

The API 37 implementation should test each of these paths rather than treating
all networking as one feature:

| Path | Expected impact |
| --- | --- |
| Internet peers, HTTP trackers, and UDP trackers | Should continue through `INTERNET`; verify with LAN permission denied |
| DHT traffic to public nodes | Should continue through `INTERNET`; verify UDP behavior |
| SSDP/UPnP router discovery and port mapping | Requires LAN access |
| Multicast and broadcast operations | Requires LAN access |
| Direct connections to peers on the same LAN | Requires LAN access |
| Incoming connections from LAN peers | Requires LAN access |
| ChromeOS/extension companion HTTP and WebSocket access over LAN | Requires LAN access |
| Same-device loopback connections | Verify against the final Android 17 behavior; do not assume |

Relevant implementation areas include:

- `android/app/src/main/AndroidManifest.xml`
- `android/app/src/main/java/com/jstorrent/app/MainActivity.kt`
- `android/app/src/main/java/com/jstorrent/app/NativeStandaloneActivity.kt`
- `android/app/src/main/java/com/jstorrent/app/ui/screens/NetworkSettingsScreen.kt`
- `android/io-core/src/main/java/com/jstorrent/io/socket/TcpSocketService.kt`
- `android/io-core/src/main/java/com/jstorrent/io/socket/UdpSocketManagerImpl.kt`
- `android/io-core/src/main/java/com/jstorrent/io/socket/UdpConnection.kt`
- `android/companion-server/src/main/java/com/jstorrent/companion/server/CompanionHttpServer.kt`
- `packages/engine/src/port-mapping/ssdp-client.ts`

## Proposed Implementation

### 1. Add a Permission Coordinator

Create one application-level abstraction that:

- Returns granted on Android versions and target SDK combinations where the
  permission is not enforced.
- Checks `ACCESS_LOCAL_NETWORK` on Android 17 when JSTorrent targets API 37.
- Exposes granted, denied, and permanently-denied states to both Android entry
  modes.
- Requests the permission through an activity using the Activity Result API.
- Detects permission changes when an activity resumes.

Services and socket classes must not attempt to show permission UI themselves.
They should report a permission-related failure to the application layer.

### 2. Use Contextual UX

Do not claim that JSTorrent is entirely unusable without LAN permission.
Internet torrent traffic should continue to work.

Show a short rationale immediately before a LAN-dependent operation, such as:

- Enabling automatic router port mapping.
- Starting or pairing companion mode for another LAN device.
- Enabling a feature that depends on LAN discovery.

The rationale should explain which feature will be limited and provide:

- **Continue** to open the Android permission prompt.
- **Not now** to continue with LAN-dependent features disabled.
- **Open settings** after a permanent denial or later revocation.

If product testing shows that companion mode cannot serve its primary purpose
without the permission, that mode can use a blocking permission screen while
leaving native standalone torrenting available.

### 3. Guard LAN Operations

Before starting LAN-dependent operations:

- Skip SSDP/UPnP discovery when permission is unavailable and expose an
  actionable status in Network Settings.
- Prevent companion mode from advertising or accepting LAN connections until
  permission is granted.
- Convert permission-related TCP and UDP failures into a typed error that can
  reach the UI.
- Retry only after permission is granted; avoid background prompt loops.

Keep public peer, tracker, and DHT traffic operational when permission is
denied. Do not broadly disable the socket layer.

### 4. Add Manifest and UI Changes with the API 37 Target

In the API 37 target update:

- Add `android.permission.ACCESS_LOCAL_NETWORK` to the app manifest.
- Add localized rationale, denied-state, and settings text.
- Add a permission/status row to Network Settings.
- Cover both `MainActivity` companion mode and
  `NativeStandaloneActivity`.
- Preserve the notification permission flow as a separate concern.

## Testing Before API 37

Android 16 can exercise the restricted state before the production permission
is added:

```bash
adb shell am compat enable RESTRICT_LOCAL_NETWORK com.jstorrent.app
adb reboot
```

Follow the current Android 16 documentation for temporarily granting and
revoking the test permission. Do not ship that temporary API 36 test setup as
the API 37 implementation.

At minimum, record results for:

- A public torrent download with LAN access restricted.
- Public TCP and UDP peers.
- HTTP and UDP trackers.
- DHT bootstrap and peer discovery.
- SSDP/UPnP discovery and port mapping.
- Companion pairing and HTTP/WebSocket traffic from another LAN device.
- Incoming and outgoing peers on the same LAN.

## API 37 Release Acceptance Criteria

- Fresh install: allow, deny, and permanent-denial paths behave predictably.
- Upgrade from the API 36 release does not crash or silently stall.
- Revoking the permission in system settings stops only LAN-dependent features.
- Restoring the permission resumes those features without reinstalling.
- Ordinary internet torrenting works while the permission is denied.
- LAN socket errors are visible and actionable rather than reported as generic
  torrent failures.
- No permission prompt is initiated from a background service.
- Unit, Compose, and API 37 instrumented tests cover the coordinator and both
  Android entry modes.
- The release bundle manifest declares the permission and target SDK 37.

## References

- [Android local network permission](https://developer.android.com/privacy-and-security/local-network-permission)
- [Android 17 behavior changes for apps targeting API 37](https://developer.android.com/about/versions/17/behavior-changes-17)
- [Google Play target API requirements](https://support.google.com/googleplay/android-developer/answer/11926878)
