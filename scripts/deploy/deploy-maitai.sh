#!/usr/bin/env bash
# deploy-maitai.sh — One-command pull, build, launch for maitai hardware testing
#
# Consolidates the full maitai deployment workflow:
#   1. SSH to maitai, pull latest code, clean build with all hardware features
#   2. Stop any running daemon
#   3. Start new daemon with correct hardware config
#   4. Launch local GUI connecting to maitai
#
# Usage:
#   bash scripts/deploy/deploy-maitai.sh                           # Full deploy from main
#   bash scripts/deploy/deploy-maitai.sh --branch feat/my-feature  # Deploy a feature branch
#   bash scripts/deploy/deploy-maitai.sh --with-db                 # Enable SurrealDB persistence
#   bash scripts/deploy/deploy-maitai.sh --gui-only                # Just launch GUI (daemon running)
#   bash scripts/deploy/deploy-maitai.sh --skip-build --daemon-only  # Restart daemon, skip build
#
# See --help for all options.

set -euo pipefail

# ============================================================================
# Source shared deploy library
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy-common.sh
source "${SCRIPT_DIR}/deploy-common.sh"

# ============================================================================
# Configuration
# ============================================================================
MAITAI_USER="${MAITAI_USER:-maitai}"
MAITAI_HOST="${MAITAI_HOST:-maitai-eos}"  # Tailscale hostname
MAITAI_SSH="${MAITAI_SSH:-${MAITAI_USER}@${MAITAI_HOST}}"
DEPLOY_SSH="$MAITAI_SSH"
REMOTE_DIR="${REMOTE_DIR:-/home/${MAITAI_USER}/code/rust-daq}"
DAEMON_PORT=50051
REMOTE_LOG="/tmp/rust-daq-daemon.log"

# ============================================================================
# Defaults
# ============================================================================
BRANCH="main"
WITH_DB=false
SKIP_BUILD=false
SKIP_GUI=false
GUI_ONLY=false
RUNTIME_MODE=""

# ============================================================================
# Parse arguments
# ============================================================================
print_help() {
    cat <<'HELP'
deploy-maitai.sh — One-command pull, build, launch for maitai hardware testing

OPTIONS:
  --branch <name>         Branch to checkout on maitai (default: main)
  --with-db               Enable SurrealDB persistence (--db-path data/surrealdb-maitai)
  --skip-build            Skip remote build (just restart daemon + launch GUI)
  --skip-gui              Don't launch local GUI (deploy daemon only)
  --daemon-only           Alias for --skip-gui
  --gui-only              Skip all remote steps, just launch local GUI
  --runtime-mode <mode>   Override daemon runtime mode (mock|native|universal|hybrid-db)
  --help                  Show this help

EXAMPLES:
  # Full deploy: pull main, clean build, start daemon, launch GUI
  bash scripts/deploy/deploy-maitai.sh

  # Deploy a feature branch with SurrealDB
  bash scripts/deploy/deploy-maitai.sh --branch feat/graph-plan --with-db

  # Just restart daemon (no build, no GUI)
  bash scripts/deploy/deploy-maitai.sh --skip-build --daemon-only

  # Just launch GUI (daemon already running on maitai)
  bash scripts/deploy/deploy-maitai.sh --gui-only
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --branch)
            BRANCH="$2"
            shift 2
            ;;
        --with-db)
            WITH_DB=true
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --skip-gui)
            SKIP_GUI=true
            shift
            ;;
        --daemon-only)
            SKIP_GUI=true
            shift
            ;;
        --gui-only)
            GUI_ONLY=true
            shift
            ;;
        --runtime-mode)
            RUNTIME_MODE="$2"
            shift 2
            ;;
        --help|-h)
            print_help
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            print_help
            exit 1
            ;;
    esac
done

# ============================================================================
# Banner
# ============================================================================
echo -e "${BOLD}${CYAN}"
echo "╔══════════════════════════════════════════════════╗"
echo "║        rust-daq Maitai Deploy                    ║"
echo "╚══════════════════════════════════════════════════╝"
echo -e "${NC}"
echo -e "  Branch:     ${BOLD}${BRANCH}${NC}"
echo -e "  SurrealDB:  ${BOLD}$(${WITH_DB} && echo 'enabled' || echo 'disabled')${NC}"
echo -e "  Build:      ${BOLD}$(${SKIP_BUILD} && echo 'skip' || echo 'clean + release')${NC}"
echo -e "  GUI:        ${BOLD}$(${SKIP_GUI} && echo 'skip' || echo 'launch locally')${NC}"
if [[ -n "$RUNTIME_MODE" ]]; then
    echo -e "  Mode:       ${BOLD}${RUNTIME_MODE}${NC}"
fi
echo ""

# ============================================================================
# Phase 0: Connectivity check
# ============================================================================
if ! $GUI_ONLY; then
    deploy_check_ssh "maitai"
    deploy_check_rust

    # Check PVCAM SDK (Phase 1 validates PVCAM_SDK_DIR from maitai.env;
    # this just confirms the installation directory exists at all)
    if ! remote "test -d /opt/pvcam/sdk"; then
        fail "PVCAM SDK not found at /opt/pvcam/sdk — install Teledyne PVCAM"
    fi
    ok "PVCAM SDK found"
fi

# ============================================================================
# Phase 1: Remote pull & build
# ============================================================================
if ! $GUI_ONLY && ! $SKIP_BUILD; then
    step "Phase 1: Pull & build on maitai (branch: ${BRANCH})"

    deploy_fetch_and_checkout "$BRANCH" "origin"

    info "Verifying PVCAM SDK environment..."
    PVCAM_CHECK=$(remote "source ${REMOTE_DIR}/config/hosts/maitai.env && echo \$PVCAM_SDK_DIR" 2>/dev/null)
    if [[ -z "$PVCAM_CHECK" ]]; then
        fail "PVCAM_SDK_DIR not set on maitai. Check config/hosts/maitai.env"
    fi
    ok "PVCAM_SDK_DIR=${PVCAM_CHECK}"

    info "Clean building (release, all hardware + db-surreal-rocksdb)..."
    info "This will take several minutes..."
    # Build with maitai + db-surreal-rocksdb so the binary has full capabilities.
    # The DB is only activated at runtime when --db-path is passed.
    remote "
        source \$HOME/.cargo/env 2>/dev/null || true && \
        cd ${REMOTE_DIR} && \
        source config/hosts/maitai.env && \
        cargo clean 2>/dev/null || true && \
        cargo build --release -p bin --features maitai,db-surreal-rocksdb
    " 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done
    ok "Build complete"

    deploy_verify_binary
fi

# ============================================================================
# Phase 2: Stop old daemon
# ============================================================================
if ! $GUI_ONLY; then
    deploy_stop_daemon
fi

# ============================================================================
# Phase 3: Start new daemon
# ============================================================================
if ! $GUI_ONLY; then
    step "Phase 3: Starting new daemon"

    # Warn if --skip-build could cause a flag/binary mismatch
    if $SKIP_BUILD && $WITH_DB; then
        warn "--skip-build with --with-db: ensure the existing binary was built with db-surreal-rocksdb"
    fi

    # Build daemon command line
    DAEMON_CMD="./target/release/rust-daq-daemon daemon --port ${DAEMON_PORT}"

    if [[ -n "$RUNTIME_MODE" ]]; then
        deploy_validate_runtime_mode "$RUNTIME_MODE"
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode ${RUNTIME_MODE}"
    else
        # Default to hybrid-db (universal TOML + SurrealDB control-plane).
        # Use --runtime-mode native to fall back to legacy hardware config.
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode hybrid-db"
    fi

    if $WITH_DB; then
        DAEMON_CMD="${DAEMON_CMD} --db-path data/surrealdb-maitai"
    fi

    # Create DB data directory if needed
    if $WITH_DB; then
        remote "mkdir -p ${REMOTE_DIR}/data" 2>/dev/null || true
    fi

    deploy_start_daemon "$DAEMON_CMD" "config/hosts/maitai.env"
    deploy_wait_for_daemon
    deploy_show_startup_log
fi

# ============================================================================
# Phase 4: Launch local GUI
# ============================================================================
if ! $SKIP_GUI; then
    deploy_launch_gui "$MAITAI_HOST" "deploy-maitai.sh"
fi

# ============================================================================
# Done
# ============================================================================
echo ""
echo -e "${GREEN}${BOLD}Deploy complete.${NC}"
