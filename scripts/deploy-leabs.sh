#!/usr/bin/env bash
# deploy-leabs.sh — One-command pull, build, launch for leabs-dev hardware testing
#
# Consolidates the full leabs-dev deployment workflow:
#   1. SSH to leabs-dev, pull latest code, build with Andor SDK3 hardware features
#   2. Stop any running daemon
#   3. Start new daemon with correct hardware config
#   4. Launch local GUI connecting to leabs-dev
#
# Usage:
#   bash scripts/deploy-leabs.sh                           # Full deploy from main
#   bash scripts/deploy-leabs.sh --branch feat/my-feature  # Deploy a feature branch
#   bash scripts/deploy-leabs.sh --gui-only                # Just launch GUI (daemon running)
#   bash scripts/deploy-leabs.sh --skip-build --daemon-only  # Restart daemon, skip build
#
# See --help for all options.

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================
LEABS_SSH="leabs-dev"  # Tailscale hostname
REMOTE_DIR="/home/brian/code/rust-daq"
DAEMON_PORT=50051
REMOTE_LOG="/tmp/rust-daq-daemon.log"
ENV_FILE="config/hosts/leabs-dev.env"
HARDWARE_CONFIG="config/leabs_hardware.toml"
CARGO_FEATURES="leabs_hardware"

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
SKIP_BUILD=false
SKIP_GUI=false
GUI_ONLY=false
RUNTIME_MODE=""

# ============================================================================
# Parse arguments
# ============================================================================
print_help() {
    cat <<'HELP'
deploy-leabs.sh — One-command pull, build, launch for leabs-dev hardware testing

OPTIONS:
  --branch <name>         Branch to checkout on leabs-dev (default: main)
  --skip-build            Skip remote build (just restart daemon + launch GUI)
  --skip-gui              Don't launch local GUI (deploy daemon only)
  --daemon-only           Alias for --skip-gui
  --gui-only              Skip all remote steps, just launch local GUI
  --runtime-mode <mode>   Override daemon runtime mode (mock|native|universal|hybrid-db)
  --help                  Show this help

EXAMPLES:
  # Full deploy: pull main, build, start daemon, launch GUI
  bash scripts/deploy-leabs.sh

  # Deploy a feature branch
  bash scripts/deploy-leabs.sh --branch feat/leabs-andor-hardware

  # Just restart daemon (no build, no GUI)
  bash scripts/deploy-leabs.sh --skip-build --daemon-only

  # Just launch GUI (daemon already running on leabs-dev)
  bash scripts/deploy-leabs.sh --gui-only
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --branch)
            BRANCH="$2"
            shift 2
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
    ssh -o ConnectTimeout=10 -o BatchMode=yes "${LEABS_SSH}" "$@"
}

# ============================================================================
# Banner
# ============================================================================
echo -e "${BOLD}${CYAN}"
echo "╔══════════════════════════════════════════════════╗"
echo "║        rust-daq LEABS Deploy                     ║"
echo "╚══════════════════════════════════════════════════╝"
echo -e "${NC}"
echo -e "  Target:     ${BOLD}${LEABS_SSH}${NC}"
echo -e "  Branch:     ${BOLD}${BRANCH}${NC}"
echo -e "  Features:   ${BOLD}${CARGO_FEATURES}${NC}"
echo -e "  Build:      ${BOLD}$(${SKIP_BUILD} && echo 'skip' || echo 'release')${NC}"
echo -e "  GUI:        ${BOLD}$(${SKIP_GUI} && echo 'skip' || echo 'launch locally')${NC}"
if [[ -n "$RUNTIME_MODE" ]]; then
    echo -e "  Mode:       ${BOLD}${RUNTIME_MODE}${NC}"
fi
echo ""

# ============================================================================
# Phase 0: Connectivity & prerequisites check
# ============================================================================
if ! $GUI_ONLY; then
    step "Phase 0: Checking SSH connectivity to leabs-dev"
    if ! remote "echo ok" &>/dev/null; then
        fail "Cannot SSH to ${LEABS_SSH}. Is Tailscale running?"
    fi
    ok "SSH to ${LEABS_SSH} works"

    # Check Rust toolchain
    if ! remote "command -v rustc" &>/dev/null; then
        warn "Rust toolchain not found on leabs-dev"
        info "Install with: ssh ${LEABS_SSH} 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh'"
        fail "Rust toolchain required for remote build"
    fi
    ok "Rust toolchain available"

    # Check Andor SDK
    if ! remote "test -f /usr/local/lib/libatcore.so"; then
        fail "Andor SDK3 not found at /usr/local/lib/libatcore.so"
    fi
    ok "Andor SDK3 found"
fi

# ============================================================================
# Phase 1: Remote pull & build
# ============================================================================
if ! $GUI_ONLY && ! $SKIP_BUILD; then
    step "Phase 1: Pull & build on leabs-dev (branch: ${BRANCH})"

    info "Fetching latest code..."
    remote "cd ${REMOTE_DIR} && git fetch --all --prune" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done

    info "Checking out ${BRANCH}..."
    remote "cd ${REMOTE_DIR} && git checkout \"${BRANCH}\" && git pull github \"${BRANCH}\"" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done
    ok "On branch ${BRANCH}, up to date"

    info "Verifying Andor SDK3 environment..."
    SDK_CHECK=$(remote "source ${REMOTE_DIR}/${ENV_FILE} && echo \$ANDOR_SDK3_DIR" 2>/dev/null)
    if [[ -z "$SDK_CHECK" ]]; then
        fail "ANDOR_SDK3_DIR not set. Check ${ENV_FILE}"
    fi
    ok "ANDOR_SDK3_DIR=${SDK_CHECK}"

    info "Building (release, ${CARGO_FEATURES})..."
    info "This will take several minutes..."
    remote "
        source \$HOME/.cargo/env && \
        cd ${REMOTE_DIR} && \
        source ${ENV_FILE} && \
        cargo build --release -p bin --features ${CARGO_FEATURES}
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
        PID_LIST=$(echo "$OLD_PIDS" | tr '\n' ' ')
        info "Killing daemon process(es): ${PID_LIST}"
        remote "pkill -f 'rust-daq-daemon daemon'" 2>/dev/null || true

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

    DAEMON_CMD="./target/release/rust-daq-daemon daemon --port ${DAEMON_PORT}"

    if [[ -n "$RUNTIME_MODE" ]]; then
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode \"${RUNTIME_MODE}\""
    else
        DAEMON_CMD="${DAEMON_CMD} --hardware-config ${HARDWARE_CONFIG}"
    fi

    info "Command: ${DAEMON_CMD}"
    info "Log: ${REMOTE_LOG}"

    # Launch daemon in background via nohup
    remote "
        source \$HOME/.cargo/env && \
        cd ${REMOTE_DIR} && \
        source ${ENV_FILE} && \
        nohup ${DAEMON_CMD} > ${REMOTE_LOG} 2>&1 &
        echo \$!
    "

    # Wait for daemon to start listening
    info "Waiting for daemon to start (port ${DAEMON_PORT})..."
    DAEMON_READY=false
    for i in $(seq 1 60); do
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
        fail "Daemon failed to start within 60s. Check logs: ssh ${LEABS_SSH} 'tail -50 ${REMOTE_LOG}'"
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
    info "Connecting to http://${LEABS_SSH}:${DAEMON_PORT}"
    info "Close the GUI window or press Ctrl+C to exit"
    echo ""

    cargo run --bin rust-daq-gui -- --daemon-url "http://${LEABS_SSH}:${DAEMON_PORT}" || true

    echo ""
    echo -e "${GREEN}GUI closed. Daemon still running on leabs-dev.${NC}"
    echo -e "  Daemon log:  ${BLUE}ssh ${LEABS_SSH} 'tail -f ${REMOTE_LOG}'${NC}"
    echo -e "  Stop daemon: ${BLUE}ssh ${LEABS_SSH} 'pkill -f rust-daq-daemon'${NC}"
    echo -e "  Reconnect:   ${BLUE}bash scripts/deploy-leabs.sh --gui-only${NC}"
fi

# ============================================================================
# Done
# ============================================================================
echo ""
echo -e "${GREEN}${BOLD}Deploy complete.${NC}"
