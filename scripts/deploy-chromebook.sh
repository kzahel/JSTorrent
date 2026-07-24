#!/usr/bin/env bash
# Build and deploy the unpacked extension to the ChromeOS testbed.
#
# The first load is manual in chrome://extensions:
#   Downloads/jstorrent-extension
#
# Environment overrides:
#   CHROMEOS_TESTBED_CLI  Path to chromeos-testbed/bin/chromeos
#   CHROMEROOT_HOST      SSH alias for the ChromeOS host
#   CDP_PORT             Local CDP tunnel port
#   JSTORRENT_EXTENSION_ID

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TESTBED_CLI="${CHROMEOS_TESTBED_CLI:-$HOME/code/chromeos-testbed/bin/chromeos}"
CHROMEROOT_HOST="${CHROMEROOT_HOST:-chromeroot}"
CDP_PORT="${CDP_PORT:-9222}"
EXTENSION_ID="${JSTORRENT_EXTENSION_ID:-dbokmlpefliilbjldladbimlcfgbolhk}"

if [[ ! -x "$TESTBED_CLI" ]]; then
    echo "ChromeOS testbed CLI not found: $TESTBED_CLI" >&2
    echo "Clone https://github.com/kzahel/chromeos-testbed to ~/code/chromeos-testbed." >&2
    exit 1
fi

cd "$REPO_DIR"

echo "Checking ChromeOS testbed..."
"$TESTBED_CLI" doctor

echo "Building extension..."
pnpm build

if ! nc -z localhost "$CDP_PORT" 2>/dev/null; then
    echo "Starting Chrome DevTools tunnel on localhost:$CDP_PORT..."
    ssh -fNT -o ExitOnForwardFailure=yes \
        -L "$CDP_PORT:127.0.0.1:9222" \
        "$CHROMEROOT_HOST"
fi

echo "Deploying and reloading JSTorrent extension..."
CDP_PORT="$CDP_PORT" "$TESTBED_CLI" deploy-ext \
    extension/dist \
    --name jstorrent-extension \
    --reload "$EXTENSION_ID"

echo "Done. Unpacked extension path: Downloads/jstorrent-extension"
