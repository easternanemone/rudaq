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
#   bash scripts/deploy-maitai.sh                           # Full deploy from main
#   bash scripts/deploy-maitai.sh --branch feat/my-feature  # Deploy a feature branch
#   bash scripts/deploy-maitai.sh --with-db                 # Enable SurrealDB persistence
#   bash scripts/deploy-maitai.sh --gui-only                # Just launch GUI (daemon running)
#   bash scripts/deploy-maitai.sh --skip-build --daemon-only  # Restart daemon, skip build
#
# See --help for all options.

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================
MAITAI_USER="maitai"
MAITAI_HOST="100.117.5.12"
MAITAI_SSH="${MAITAI_USER}@${MAITAI_HOST}"
REMOTE_DIR="/home/maitai/code/rust-daq"
DAEMON_PORT=50051
REMOTE_LOG="/tmp/rust-daq-daemon.log"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

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
  bash scripts/deploy-maitai.sh

  # Deploy a feature branch with SurrealDB
  bash scripts/deploy-maitai.sh --branch feat/graph-plan --with-db

  # Just restart daemon (no build, no GUI)
  bash scripts/deploy-maitai.sh --skip-build --daemon-only

  # Just launch GUI (daemon already running on maitai)
  bash scripts/deploy-maitai.sh --gui-only
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
# Helpers
# ============================================================================
step() {
    echo ""
    echo -e "${CYAN}${BOLD}━━━ $1 ━━━${NC}"
}

ok() {
    echo -e "${GREEN}  ✓ $1${NC}"
}

warn() {
    echo -e "${YELLOW}  ⚠ $1${NC}"
}

fail() {
    echo -e "${RED}  ✗ $1${NC}"
    exit 1
}

info() {
    echo -e "${BLUE}  → $1${NC}"
}

remote() {
    # Execute a command on maitai via SSH
    # Uses -o ConnectTimeout to fail fast if host is unreachable
    ssh -o ConnectTimeout=10 -o BatchMode=yes "${MAITAI_SSH}" "$@"
}

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
    step "Phase 0: Checking SSH connectivity to maitai"
    if ! remote "echo ok" &>/dev/null; then
        fail "Cannot SSH to ${MAITAI_SSH}. Is Tailscale running?"
    fi
    ok "SSH to ${MAITAI_SSH} works"
fi

# ============================================================================
# Phase 1: Remote pull & build
# ============================================================================
if ! $GUI_ONLY && ! $SKIP_BUILD; then
    step "Phase 1: Pull & build on maitai (branch: ${BRANCH})"

    info "Fetching latest code..."
    remote "cd ${REMOTE_DIR} && git fetch --all --prune" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done

    info "Checking out ${BRANCH}..."
    remote "cd ${REMOTE_DIR} && git checkout ${BRANCH} && git pull origin ${BRANCH}" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done
    ok "On branch ${BRANCH}, up to date"

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
        cd ${REMOTE_DIR} && \
        source config/hosts/maitai.env && \
        cargo clean 2>/dev/null || true && \
        cargo build --release -p bin --features maitai,db-surreal-rocksdb
    " 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done
    ok "Build complete"

    # Verify binary exists
    remote "test -f ${REMOTE_DIR}/target/release/rust-daq-daemon" || fail "Binary not found after build"
    ok "Binary verified: ${REMOTE_DIR}/target/release/rust-daq-daemon"
fi

# ============================================================================
# Phase 2: Stop old daemon
# ============================================================================
if ! $GUI_ONLY; then
    step "Phase 2: Stopping old daemon"

    OLD_PIDS=$(remote "pgrep -f 'rust-daq-daemon daemon'" 2>/dev/null || true)
    if [[ -n "$OLD_PIDS" ]]; then
        # Format PIDs for display (may be multiple)
        PID_LIST=$(echo "$OLD_PIDS" | tr '\n' ' ')
        info "Killing daemon process(es): ${PID_LIST}"
        remote "pkill -f 'rust-daq-daemon daemon'" 2>/dev/null || true

        # Wait for graceful shutdown
        for i in $(seq 1 5); do
            if ! remote "pgrep -f 'rust-daq-daemon daemon'" &>/dev/null; then
                ok "Daemon stopped gracefully"
                break
            fi
            if [[ $i -eq 5 ]]; then
                warn "Daemon didn't stop gracefully, force killing..."
                remote "pkill -9 -f 'rust-daq-daemon daemon'" 2>/dev/null || true
                sleep 1
                ok "Daemon force-killed"
            fi
            sleep 1
        done
    else
        ok "No running daemon found"
    fi
fi

# ============================================================================
# Phase 3: Start new daemon
# ============================================================================
if ! $GUI_ONLY; then
    step "Phase 3: Starting new daemon"

    # Build daemon command line
    DAEMON_CMD="./target/release/rust-daq-daemon daemon --port ${DAEMON_PORT}"

    if [[ -n "$RUNTIME_MODE" ]]; then
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode ${RUNTIME_MODE}"
    else
        DAEMON_CMD="${DAEMON_CMD} --hardware-config config/maitai_hardware.toml"
    fi

    if $WITH_DB; then
        DAEMON_CMD="${DAEMON_CMD} --db-path data/surrealdb-maitai"
    fi

    info "Command: ${DAEMON_CMD}"
    info "Log: ${REMOTE_LOG}"

    # Create DB data directory if needed
    if $WITH_DB; then
        remote "mkdir -p ${REMOTE_DIR}/data" 2>/dev/null || true
    fi

    # Launch daemon in background via nohup
    remote "
        cd ${REMOTE_DIR} && \
        source config/hosts/maitai.env && \
        nohup ${DAEMON_CMD} > ${REMOTE_LOG} 2>&1 &
        echo \$!
    "

    # Wait for daemon to start listening
    info "Waiting for daemon to start (port ${DAEMON_PORT})..."
    DAEMON_READY=false
    for i in $(seq 1 15); do
        if remote "ss -tlnp 2>/dev/null | grep -q ':${DAEMON_PORT}'" 2>/dev/null; then
            DAEMON_READY=true
            break
        fi
        sleep 1
        printf "."
    done
    echo ""

    if $DAEMON_READY; then
        ok "Daemon listening on port ${DAEMON_PORT}"
    else
        fail "Daemon failed to start within 15s. Check logs: ssh ${MAITAI_SSH} 'tail -50 ${REMOTE_LOG}'"
    fi

    # Show startup log
    info "Daemon startup log:"
    remote "head -20 ${REMOTE_LOG}" 2>/dev/null | while IFS= read -r line; do
        echo -e "    ${line}"
    done

    NEW_PID=$(remote "pgrep -f 'rust-daq-daemon daemon'" 2>/dev/null || echo "unknown")
    ok "Daemon running (PID: ${NEW_PID})"
fi

# ============================================================================
# Phase 4: Launch local GUI
# ============================================================================
if ! $SKIP_GUI; then
    step "Phase 4: Launching local GUI"
    info "Connecting to http://${MAITAI_HOST}:${DAEMON_PORT}"
    info "Close the GUI window or press Ctrl+C to exit"
    echo ""

    # Run GUI in foreground — user closes it when done
    cargo run --bin rust-daq-gui -- --daemon-url "http://${MAITAI_HOST}:${DAEMON_PORT}" || true

    echo ""
    echo -e "${GREEN}GUI closed. Daemon still running on maitai.${NC}"
    echo -e "  Daemon log:  ${BLUE}ssh ${MAITAI_SSH} 'tail -f ${REMOTE_LOG}'${NC}"
    echo -e "  Stop daemon: ${BLUE}ssh ${MAITAI_SSH} 'pkill -f rust-daq-daemon'${NC}"
    echo -e "  Reconnect:   ${BLUE}bash scripts/deploy-maitai.sh --gui-only${NC}"
fi

# ============================================================================
# Done
# ============================================================================
echo ""
echo -e "${GREEN}${BOLD}Deploy complete.${NC}"
