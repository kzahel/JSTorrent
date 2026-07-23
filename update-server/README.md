# Update Service Configuration

This directory contains JSTorrent's product record for the external update
service hosted at `updates.jstorrent.com`.

[`jstorrent.json`](jstorrent.json) identifies the GitHub repository, the
`tauri-app-v` release-tag prefix, and the fact that releases expose Tauri
updater metadata. The update service itself is maintained outside this
repository.

Desktop release behavior and updater artifacts are documented in
[`docs/topics/releases.md`](../docs/topics/releases.md).
