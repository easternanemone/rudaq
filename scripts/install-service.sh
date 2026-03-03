#!/usr/bin/env bash
# Install, uninstall, or check the rust-daq-daemon systemd user service.
#
# Usage:
#   bash scripts/install-service.sh install    # copy unit, daemon-reload, enable
#   bash scripts/install-service.sh uninstall  # stop, disable, remove, daemon-reload
#   bash scripts/install-service.sh status     # systemctl --user status
#
# The service unit template lives in config/systemd/rust-daq-daemon.service.
# Install copies it to ~/.config/systemd/user/ and enables (but does not start)
# the service.  Use `systemctl --user start rust-daq-daemon` when ready.

set -euo pipefail

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
SERVICE_NAME="rust-daq-daemon"
UNIT_FILE="${SERVICE_NAME}.service"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SOURCE_UNIT="${PROJECT_ROOT}/config/systemd/${UNIT_FILE}"
TARGET_DIR="${HOME}/.config/systemd/user"
TARGET_UNIT="${TARGET_DIR}/${UNIT_FILE}"

# Colors (disable if stdout is not a terminal)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' NC=''
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
info()  { echo -e "${GREEN}[info]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
err()   { echo -e "${RED}[error]${NC} $*" >&2; }

require_linux() {
    if [[ "$(uname -s)" != "Linux" ]]; then
        err "systemd services are only supported on Linux."
        exit 1
    fi
}

require_source_unit() {
    if [[ ! -f "$SOURCE_UNIT" ]]; then
        err "Service unit not found: ${SOURCE_UNIT}"
        err "Run this script from the rust-daq project root."
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Subcommands
# ---------------------------------------------------------------------------
do_install() {
    require_linux
    require_source_unit

    info "Installing ${UNIT_FILE} -> ${TARGET_UNIT}"

    mkdir -p "$TARGET_DIR"
    cp "$SOURCE_UNIT" "$TARGET_UNIT"

    info "Reloading systemd user daemon..."
    systemctl --user daemon-reload

    info "Enabling ${SERVICE_NAME} (start on login)..."
    systemctl --user enable "$SERVICE_NAME"

    echo ""
    info "Service installed and enabled."
    info "Start it now with:  systemctl --user start ${SERVICE_NAME}"
    info "View logs with:     journalctl --user -u ${SERVICE_NAME} -f"
    echo ""
    warn "If you need host-specific environment variables (PVCAM_SDK_DIR, serial ports, etc.),"
    warn "edit ${TARGET_UNIT} and uncomment/adjust the EnvironmentFile line."
}

do_uninstall() {
    require_linux

    info "Stopping ${SERVICE_NAME}..."
    systemctl --user stop "$SERVICE_NAME" 2>/dev/null || true

    info "Disabling ${SERVICE_NAME}..."
    systemctl --user disable "$SERVICE_NAME" 2>/dev/null || true

    if [[ -f "$TARGET_UNIT" ]]; then
        info "Removing ${TARGET_UNIT}"
        rm -f "$TARGET_UNIT"
    else
        warn "Unit file not found at ${TARGET_UNIT} (already removed?)"
    fi

    info "Reloading systemd user daemon..."
    systemctl --user daemon-reload

    echo ""
    info "Service uninstalled."
}

do_status() {
    require_linux
    systemctl --user status "$SERVICE_NAME" || true
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
usage() {
    echo "Usage: $0 {install|uninstall|status}"
    echo ""
    echo "Subcommands:"
    echo "  install    Copy service unit, daemon-reload, enable"
    echo "  uninstall  Stop, disable, remove unit, daemon-reload"
    echo "  status     Show systemctl --user status"
    exit 1
}

if [[ $# -lt 1 ]]; then
    usage
fi

case "$1" in
    install)   do_install   ;;
    uninstall) do_uninstall ;;
    status)    do_status    ;;
    -h|--help) usage        ;;
    *)
        err "Unknown subcommand: $1"
        usage
        ;;
esac
