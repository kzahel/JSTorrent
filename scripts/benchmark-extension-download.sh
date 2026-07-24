#!/bin/bash
# Benchmark download speed using Chrome extension on ChromeOS.
#
# Prerequisites:
# 1. 1GB seeder running: pnpm seed-for-test --size 1gb
#
# Note: Extension is automatically deployed at start.
#
# This script handles SSH tunnel setup automatically.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PYTHON_DIR="$PROJECT_ROOT/packages/engine/integration/python"

# Default CDP port
CDP_PORT="${CDP_PORT:-9222}"
CHROMEROOT_HOST="${CHROMEROOT_HOST:-chromeroot}"
TUNNEL_STARTED=0

echo "=== Extension Download Benchmark ==="

# Deploy extension to ensure it's up to date
echo "Deploying extension to Chromebook..."
"$SCRIPT_DIR/deploy-chromebook.sh"

# Check if SSH tunnel is already running
if ! nc -z localhost "$CDP_PORT" 2>/dev/null; then
    echo "Starting SSH tunnel for CDP (port $CDP_PORT)..."
    ssh -f -N -L "$CDP_PORT:127.0.0.1:9222" "$CHROMEROOT_HOST"
    TUNNEL_STARTED=1
    sleep 1
else
    echo "SSH tunnel already active on port $CDP_PORT"
fi

# Cleanup tunnel on exit if we started it
cleanup() {
    if [ "$TUNNEL_STARTED" = "1" ]; then
        echo "Stopping SSH tunnel..."
        pkill -f "ssh.*-L $CDP_PORT.*$CHROMEROOT_HOST" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Run the benchmark
cd "$PYTHON_DIR"
uv run python benchmark_extension_download.py "$@"
