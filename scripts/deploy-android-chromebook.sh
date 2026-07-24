#!/usr/bin/env bash
# Build and install the Android app on the ChromeOS testbed.
#
# Usage:
#   ./scripts/deploy-android-chromebook.sh
#   ./scripts/deploy-android-chromebook.sh release
#   ./scripts/deploy-android-chromebook.sh --forward
#
# --forward maps Android localhost:<port> through ChromeOS to the development
# server on this machine.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TESTBED_CLI="${CHROMEOS_TESTBED_CLI:-$HOME/code/chromeos-testbed/bin/chromeos}"
CHROMEROOT_HOST="${CHROMEROOT_HOST:-chromeroot}"
DEV_SERVER_PORT="${DEV_SERVER_PORT:-3000}"
BUILD_TYPE="debug"
SETUP_FORWARD=false

for arg in "$@"; do
    case "$arg" in
        --forward|-f) SETUP_FORWARD=true ;;
        release) BUILD_TYPE="release" ;;
        debug) BUILD_TYPE="debug" ;;
        *)
            echo "Unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

if [[ ! -x "$TESTBED_CLI" ]]; then
    echo "ChromeOS testbed CLI not found: $TESTBED_CLI" >&2
    echo "Clone https://github.com/kzahel/chromeos-testbed to ~/code/chromeos-testbed." >&2
    exit 1
fi

echo "Checking ChromeOS testbed..."
"$TESTBED_CLI" doctor

cd "$REPO_DIR/android"

echo "Building $BUILD_TYPE APK..."
if [[ "$BUILD_TYPE" == "release" ]]; then
    ./gradlew assembleRelease
    APK_PATH="$PWD/app/build/outputs/apk/release/app-release.apk"
else
    ./gradlew assembleDebug
    APK_PATH="$PWD/app/build/outputs/apk/debug/app-debug.apk"
fi

echo "Installing APK through ChromeOS ARCVM ADB..."
"$TESTBED_CLI" install-apk "$APK_PATH" --authorize

if [[ "$SETUP_FORWARD" == true ]]; then
    echo "Connecting ARCVM ADB..."
    "$TESTBED_CLI" adb-connect

    if pgrep -f \
        "ssh.*-R $DEV_SERVER_PORT:localhost:$DEV_SERVER_PORT.*$CHROMEROOT_HOST" \
        >/dev/null; then
        echo "SSH reverse tunnel is already running."
    else
        echo "Starting reverse tunnel through $CHROMEROOT_HOST..."
        ssh -fNT -o ExitOnForwardFailure=yes \
            -R "$DEV_SERVER_PORT:localhost:$DEV_SERVER_PORT" \
            "$CHROMEROOT_HOST"
    fi

    ssh "$CHROMEROOT_HOST" \
        "export PATH=/bin:/usr/bin:/usr/local/bin:\$PATH; \
         adb -s 127.0.0.1:5555 reverse \
             tcp:$DEV_SERVER_PORT tcp:$DEV_SERVER_PORT"

    echo "Android localhost:$DEV_SERVER_PORT now reaches this development machine."
fi

echo "Done. Android app deployed and installed."
