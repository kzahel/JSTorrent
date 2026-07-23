#!/bin/bash
set -euo pipefail

# JSTorrent I/O Daemon installer for ChromeOS Crostini
# Usage: curl -fsSL https://jstorrent.com/install-crostini.sh | bash
#
# For ChromeOS devices without Play Store / ARC support (ChromeOS Flex, etc.).
# Installs the standalone io-daemon binary and creates a systemd user service
# so the JSTorrent Chrome extension can use it for I/O.
#
# Options:
#   --uninstall    Remove the daemon, service, and config
#   --version X    Install a specific version (e.g., --version 0.2.1)

FALLBACK_TAG="v0.2.1"
REPO="kzahel/jstorrent"
SERVICE_NAME="jstorrent-io"
BINARY_NAME="jstorrent-io-daemon"
CHECKSUMS_NAME="SHA256SUMS"
CHECKSUMS_FALLBACK_BASE_URL="${JSTORRENT_CHECKSUMS_FALLBACK_BASE_URL:-https://jstorrent.com/checksums}"
INSTALL_DIR="$HOME/.local/bin"
SERVICE_DIR="$HOME/.config/systemd/user"
CONFIG_DIR="$HOME/.config/jstorrent-standalone"
DOWNLOAD_ROOT="$HOME/Downloads"

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BOLD='' NC=''
fi

info()  { echo -e "${GREEN}==>${NC} ${BOLD}$*${NC}"; }
warn()  { echo -e "${YELLOW}warning:${NC} $*"; }
error() { echo -e "${RED}error:${NC} $*" >&2; }

download_verified_asset() {
    local asset_name="$1"
    local destination="$2"
    local checksums_file="${TMP_DIR}/${CHECKSUMS_NAME}"

    if ! command -v sha256sum >/dev/null 2>&1; then
        error "sha256sum is required to verify release downloads."
        return 1
    fi

    if [ ! -s "$checksums_file" ]; then
        info "Downloading checksum manifest..."
        if ! curl -fSL --progress-bar "${BASE_URL}/${CHECKSUMS_NAME}" -o "$checksums_file"; then
            local fallback_url="${CHECKSUMS_FALLBACK_BASE_URL}/tauri-app-${TAG}-${CHECKSUMS_NAME}"
            warn "Release checksum manifest is unavailable; checking the bootstrap manifest."
            if ! curl -fSL --progress-bar "$fallback_url" -o "$checksums_file"; then
                error "Checksum manifests are unavailable; refusing an unverified install."
                return 1
            fi
        fi
    fi

    local expected
    expected=$(awk -v name="$asset_name" '$2 == name { print $1; exit }' "$checksums_file")
    if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
        error "No valid checksum was published for ${asset_name}."
        return 1
    fi

    info "Downloading ${asset_name}..."
    if ! curl -fSL --progress-bar "${BASE_URL}/${asset_name}" -o "$destination"; then
        rm -f "$destination"
        error "Failed to download ${asset_name}."
        return 1
    fi

    local actual
    actual=$(sha256sum "$destination" | awk '{ print $1 }')
    if [ "${actual,,}" != "${expected,,}" ]; then
        rm -f "$destination"
        error "Checksum verification failed for ${asset_name}; refusing to install it."
        return 1
    fi

    info "Verified ${asset_name}."
}

if [ "${JSTORRENT_INSTALLER_LIB_ONLY:-}" = "1" ]; then
    return 0 2>/dev/null || exit 0
fi

# --- Uninstall ---
uninstall() {
    info "Uninstalling JSTorrent I/O Daemon..."

    if command -v systemctl &>/dev/null; then
        systemctl --user stop "${SERVICE_NAME}.service" 2>/dev/null || true
        systemctl --user disable "${SERVICE_NAME}.service" 2>/dev/null || true
        rm -f "${SERVICE_DIR}/${SERVICE_NAME}.service"
        systemctl --user daemon-reload
    fi

    rm -f "${INSTALL_DIR}/${BINARY_NAME}"

    if [ -d "$CONFIG_DIR" ]; then
        rm -rf "$CONFIG_DIR"
        info "Removed config: $CONFIG_DIR"
    fi

    info "JSTorrent I/O Daemon has been uninstalled."
}

# --- Parse arguments ---
REQUESTED_VERSION=""

while [ $# -gt 0 ]; do
    case "$1" in
        --uninstall) uninstall; exit 0 ;;
        --version)   REQUESTED_VERSION="$2"; shift ;;
        *)           warn "Unknown option: $1" ;;
    esac
    shift
done

# --- OS check ---
if [[ "$(uname -s)" != "Linux" ]]; then
    error "This script is for Linux only (ChromeOS Crostini)."
    exit 1
fi

# --- Architecture ---
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)  TRIPLE="x86_64-unknown-linux-gnu" ;;
    aarch64) TRIPLE="aarch64-unknown-linux-gnu" ;;
    *)
        error "Unsupported architecture: $ARCH (supported: x86_64, aarch64)"
        exit 1
        ;;
esac

# --- Fetch latest release tag ---
if [ -n "$REQUESTED_VERSION" ]; then
    TAG="v${REQUESTED_VERSION#v}"
    info "Using requested version: $TAG"
else
    info "Checking for latest release..."
    TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases" 2>/dev/null | \
        grep -o '"tag_name": "tauri-app-v[^"]*"' | head -1 | \
        sed 's/.*tauri-app-\(v[^"]*\)".*/\1/' || echo "")

    if [ -z "$TAG" ]; then
        warn "Could not fetch latest release, using fallback: $FALLBACK_TAG"
        TAG="$FALLBACK_TAG"
    else
        info "Latest release: $TAG"
    fi
fi

VERSION="${TAG#v}"
BASE_URL="https://github.com/${REPO}/releases/download/tauri-app-${TAG}"

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# --- Download binary ---
ASSET_NAME="${BINARY_NAME}-${TRIPLE}"
DOWNLOAD_URL="${BASE_URL}/${ASSET_NAME}"

if ! download_verified_asset "$ASSET_NAME" "${TMP_DIR}/${ASSET_NAME}"; then
    error "Unable to obtain a verified ${ASSET_NAME}."
    echo "  URL: $DOWNLOAD_URL"
    echo ""
    echo "  This may mean the binary hasn't been published for this version yet."
    echo "  Try specifying a version: curl -fsSL https://jstorrent.com/install-crostini.sh | bash -s -- --version 0.2.1"
    exit 1
fi

# --- Install binary ---
mkdir -p "$INSTALL_DIR"
mv "${TMP_DIR}/${ASSET_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
info "Installed to ${INSTALL_DIR}/${BINARY_NAME}"

# --- Ensure download directory exists ---
mkdir -p "$DOWNLOAD_ROOT"

# --- systemd service ---
if ! command -v systemctl &>/dev/null; then
    warn "systemd not found. The daemon won't auto-start."
    echo ""
    echo "  Run manually:"
    echo "    ${INSTALL_DIR}/${BINARY_NAME} --standalone --download-root ${DOWNLOAD_ROOT}"
    echo ""
    echo "  The JSTorrent Chrome extension will automatically detect this daemon."
    exit 0
fi

info "Setting up systemd user service..."

mkdir -p "$SERVICE_DIR"
cat > "${SERVICE_DIR}/${SERVICE_NAME}.service" <<EOF
[Unit]
Description=JSTorrent I/O Daemon
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=%h/.local/bin/jstorrent-io-daemon --standalone --download-root %h/Downloads
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
EOF

# Enable lingering so the user service manager starts at boot
loginctl enable-linger "$USER" 2>/dev/null || warn "Could not enable lingering (service won't auto-start after reboot)"

# Reload, enable, and (re)start
systemctl --user daemon-reload
systemctl --user enable "${SERVICE_NAME}.service" 2>/dev/null
systemctl --user restart "${SERVICE_NAME}.service"

# --- Verify ---
sleep 1
if systemctl --user is-active --quiet "${SERVICE_NAME}.service"; then
    info "JSTorrent I/O Daemon is running!"
    if curl -sf http://localhost:7800/health >/dev/null 2>&1; then
        info "Health check passed (port 7800)"
    fi
else
    warn "Service may not be running yet. Check with:"
    echo "  systemctl --user status ${SERVICE_NAME}"
fi

echo ""
info "Installation complete! (v${VERSION})"
echo "  The JSTorrent Chrome extension will automatically detect this daemon."
echo "  Downloaded files will appear in: ${DOWNLOAD_ROOT}"
echo ""
echo "  Useful commands:"
echo "    systemctl --user status ${SERVICE_NAME}    # Check status"
echo "    journalctl --user -u ${SERVICE_NAME} -f    # View logs"
echo "    curl -fsSL https://jstorrent.com/install-crostini.sh | bash  # Update"
echo ""
echo "  Uninstall:"
echo "    curl -fsSL https://jstorrent.com/install-crostini.sh | bash -s -- --uninstall"
echo ""
echo "  Note: After a ChromeOS reboot, open the Terminal app (or any Linux app)"
echo "  once to start Crostini. The daemon will start automatically after that."
