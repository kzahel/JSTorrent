# Node.js Daemon-Backed Engine Client

A Node.js BitTorrent engine client that runs on Chromebook Crostini and uses an external daemon (Android companion server or Rust io-daemon) for all network/disk I/O.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Chromebook Crostini                         │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Node.js Engine Client                                    │  │
│  │  ┌────────────────┐    ┌─────────────────────────────┐   │  │
│  │  │ HTTP RPC Server│◄───│ Python test client / curl   │   │  │
│  │  │ (port 3000)    │    └─────────────────────────────┘   │  │
│  │  └───────┬────────┘                                      │  │
│  │          │                                                │  │
│  │  ┌───────▼────────┐                                      │  │
│  │  │   BtEngine     │  (torrent logic, piece management)   │  │
│  │  └───────┬────────┘                                      │  │
│  │          │                                                │  │
│  │  ┌───────▼────────┐                                      │  │
│  │  │DaemonConnection│  WebSocket binary protocol           │  │
│  │  └───────┬────────┘                                      │  │
│  └──────────┼───────────────────────────────────────────────┘  │
│             │                                                   │
│             ▼                                                   │
│  ┌──────────────────────┐    OR    ┌────────────────────────┐  │
│  │ Android Companion    │          │ Rust io-daemon         │  │
│  │ 100.115.92.2:7800    │          │ localhost:7800         │  │
│  │ (ARC container)      │          │ (standalone mode)      │  │
│  └──────────────────────┘          └────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## Quick Start

### 1. Get credentials from Android app

```bash
# Read auth credentials from Android shared preferences
ADB=/home/graehlarts/android-sdk/platform-tools/adb
$ADB shell "run-as com.jstorrent.app cat shared_prefs/jstorrent_auth.xml"
```

This outputs XML with:
- `auth_token` - The auth token
- `extension_id` - Extension ID for auth headers
- `install_id` - Install ID for auth headers

### 2. Create .env file (gitignored)

```bash
cat > .env << 'EOF'
JST_HOST=100.115.92.2
JST_PORT=7800
JST_TOKEN=<auth_token from above>
JST_EXTENSION_ID=<extension_id from above>
JST_INSTALL_ID=<install_id from above>
RPC_PORT=3000
EOF
```

### 3. Run the daemon client

```bash
cd packages/engine
source ../../.env
npx tsx src/cmd/run-daemon-rpc.ts
```

Output:
```
Connecting to daemon at 100.115.92.2:7800...
Fetching daemon status...
Daemon status: port=7800, ioPort=7806, paired=true
WebSocket connected on ioPort 7806
Fetched 1 storage roots:
  - jstorrent (46f460d74735e5cc)
Engine created, listening on port 6881
RPC_PORT=3000
HTTP RPC Server listening on port 3000
```

## HTTP RPC API

### Engine Status
```bash
curl http://localhost:3000/engine/status
# {"ok":true,"running":true,"version":"1.0.0","port":6881,"torrents":[],"daemonConnected":true}
```

### Storage Roots
```bash
curl http://localhost:3000/engine/roots
# {"ok":true,"roots":[{"key":"46f460d74735e5cc","label":"jstorrent","path":"content://..."}]}
```

### Add Torrent (magnet)
```bash
curl -X POST http://localhost:3000/torrent/add \
  -H "Content-Type: application/json" \
  -d '{"type":"magnet","data":"magnet:?xt=urn:btih:..."}'
# {"ok":true,"id":"dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c"}
```

### Add Torrent (file)
```bash
# Base64-encode the .torrent file
TORRENT_B64=$(base64 -w0 file.torrent)
curl -X POST http://localhost:3000/torrent/add \
  -H "Content-Type: application/json" \
  -d "{\"type\":\"file\",\"data\":\"$TORRENT_B64\"}"
```

### Torrent Status
```bash
curl http://localhost:3000/torrent/<infohash>/status
# {"ok":true,"id":"...","state":"downloading","progress":0.5,"downloadRate":500000,"peers":10}
```

### List Peers
```bash
curl http://localhost:3000/torrent/<infohash>/peers
```

### Remove Torrent
```bash
# Remove torrent only (keep downloaded files)
curl -X POST http://localhost:3000/torrent/<infohash>/remove

# Remove torrent and delete downloaded files
curl -X POST http://localhost:3000/torrent/<infohash>/remove \
  -H "Content-Type: application/json" \
  -d '{"deleteData":true}'
```

### Shutdown
```bash
curl -X POST http://localhost:3000/shutdown
```

## Example: Download Big Buck Bunny

```bash
#!/bin/bash
# download-big-buck-bunny.sh

cd /path/to/jstorrent/packages/engine
source ../../.env

# Start daemon client in background
npx tsx src/cmd/run-daemon-rpc.ts &
DAEMON_PID=$!
sleep 8

# Add Big Buck Bunny torrent
MAGNET='magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny&tr=udp%3A%2F%2Fexplodie.org%3A6969&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969&tr=udp%3A%2F%2Ftracker.empire-js.us%3A1337&tr=udp%3A%2F%2Ftracker.leechers-paradise.org%3A6969&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337&tr=wss%3A%2F%2Ftracker.btorrent.xyz&tr=wss%3A%2F%2Ftracker.fastcast.nz&tr=wss%3A%2F%2Ftracker.openwebtorrent.com&ws=https%3A%2F%2Fwebtorrent.io%2Ftorrents%2F&xs=https%3A%2F%2Fwebtorrent.io%2Ftorrents%2Fbig-buck-bunny.torrent'

echo "Adding torrent..."
curl -s -X POST http://localhost:3000/torrent/add \
  -H "Content-Type: application/json" \
  -d "{\"type\":\"magnet\",\"data\":\"$MAGNET\"}"
echo ""

HASH="dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c"

# Poll status until complete
while true; do
  STATUS=$(curl -s http://localhost:3000/torrent/$HASH/status)
  PROGRESS=$(echo "$STATUS" | grep -o '"progress":[0-9.]*' | cut -d: -f2)
  RATE=$(echo "$STATUS" | grep -o '"downloadRate":[0-9]*' | cut -d: -f2)
  PEERS=$(echo "$STATUS" | grep -o '"peers":[0-9]*' | cut -d: -f2)

  # Convert to percentage and MB/s
  PCT=$(echo "$PROGRESS * 100" | bc -l 2>/dev/null | cut -d. -f1)
  MBPS=$(echo "scale=2; $RATE / 1048576" | bc -l 2>/dev/null)

  echo "Progress: ${PCT:-0}% | Speed: ${MBPS:-0} MB/s | Peers: ${PEERS:-0}"

  # Check if complete
  if [ "$(echo "$PROGRESS >= 1.0" | bc -l 2>/dev/null)" = "1" ]; then
    echo "Download complete!"
    break
  fi

  sleep 5
done

# Cleanup
curl -s -X POST http://localhost:3000/shutdown
wait $DAEMON_PID 2>/dev/null
```

## CLI Options

```
Usage: run-daemon-rpc.ts [options]

Options:
  --host <ip>         Daemon host (default: 127.0.0.1, env: JST_HOST)
  --port <port>       Daemon port (default: 7800, env: JST_PORT)
  --token <token>     Auth token (required, env: JST_TOKEN)
  --extension-id <id> Extension ID for auth (env: JST_EXTENSION_ID)
  --install-id <id>   Install ID for auth (env: JST_INSTALL_ID)
  --rpc-port <port>   HTTP RPC server port (default: 3000, env: RPC_PORT)
  --session-path <p>  Path to session file (default: ~/.config/jstorrent-node-client/session.json)
  --no-session        Disable session persistence (stateless mode for benchmarking)
  --help, -h          Show help
```

## Benchmarking

For repeatable download speed tests, use stateless mode:

```bash
./scripts/benchmark-daemon-download.sh
```

Or manually:
```bash
# Start with --no-session to avoid restoring previous state
npx tsx src/cmd/run-daemon-rpc.ts --no-session

# After download, remove with data cleanup
curl -X POST http://localhost:3000/torrent/<hash>/remove \
  -H "Content-Type: application/json" \
  -d '{"deleteData":true}'
```

## Implementation Files

| File | Description |
|------|-------------|
| `packages/engine/src/cmd/run-daemon-rpc.ts` | CLI entry point with HTTP RPC server |
| `packages/engine/src/adapters/daemon/daemon-client.ts` | Helper for fetching roots, status |
| `packages/engine/src/presets/daemon.ts` | Engine preset accepting pre-connected DaemonConnection |

## Notes

- ARC container IP is typically `100.115.92.2`
- Android companion uses port 7800 (HTTP) and 7801+ (WebSocket IO)
- The client auto-discovers the WebSocket port via `/status` endpoint
- Session data is stored in `~/.config/jstorrent-node-client/session.json`
- Downloaded files go to the Android app's configured storage root
