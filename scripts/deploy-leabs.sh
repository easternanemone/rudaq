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
LEABS_USER="${LEABS_USER:-brian}"
LEABS_HOST="${LEABS_HOST:-leabs-dev}"  # Tailscale hostname
LEABS_SSH="${LEABS_SSH:-${LEABS_USER}@${LEABS_HOST}}"
REMOTE_DIR="${REMOTE_DIR:-/home/${LEABS_USER}/code/rust-daq}"
DAEMON_PORT=50051
REMOTE_LOG="/tmp/rust-daq-daemon.log"
ENV_FILE="config/hosts/leabs-dev.env"
HARDWARE_CONFIG="config/leabs_hardware.toml"
CARGO_FEATURES="leabs_hardware,db-surreal-rocksdb"

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
WITH_DB=true
SKIP_BUILD=false
SKIP_GUI=false
GUI_ONLY=false
WASM_GUI=false
RUNTIME_MODE=""

# ============================================================================
# Parse arguments
# ============================================================================
print_help() {
    cat <<'HELP'
deploy-leabs.sh — One-command pull, build, launch for leabs-dev hardware testing

OPTIONS:
  --branch <name>         Branch to checkout on leabs-dev (default: main)
  --with-db               Enable SurrealDB persistence (default: enabled)
  --no-db                 Disable SurrealDB persistence (build without db feature)
  --skip-build            Skip remote build (just restart daemon + launch GUI)
  --skip-gui              Don't launch local GUI (deploy daemon only)
  --daemon-only           Alias for --skip-gui
  --gui-only              Skip all remote steps, just launch local GUI
  --wasm-gui              Build and serve WASM GUI on leabs-dev (port 8080)
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
        --with-db)
            WITH_DB=true
            shift
            ;;
        --no-db)
            WITH_DB=false
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
        --wasm-gui)
            WASM_GUI=true
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
# Adjust features based on DB flag
if ! $WITH_DB; then
    CARGO_FEATURES="leabs_hardware"
fi

echo -e "  Target:     ${BOLD}${LEABS_SSH}${NC}"
echo -e "  Branch:     ${BOLD}${BRANCH}${NC}"
echo -e "  Features:   ${BOLD}${CARGO_FEATURES}${NC}"
echo -e "  SurrealDB:  ${BOLD}$(${WITH_DB} && echo 'enabled' || echo 'disabled')${NC}"
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

    # Check Rust toolchain — try PATH first, then source cargo env as fallback
    # (SSH BatchMode doesn't load login profile, and rustc may be installed
    # via system packages without $HOME/.cargo/env)
    if ! remote 'command -v rustc >/dev/null 2>&1 || { [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"; command -v rustc >/dev/null 2>&1; }' &>/dev/null; then
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

    # Validate branch name to prevent shell injection via SSH
    if [[ ! "$BRANCH" =~ ^[a-zA-Z0-9._/-]+$ ]]; then
        fail "Invalid branch name '${BRANCH}' — only alphanumeric, '.', '_', '/', '-' allowed"
    fi

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

    # Warn if --skip-build could cause a flag/binary mismatch
    if $SKIP_BUILD && $WITH_DB; then
        warn "--skip-build with --with-db: ensure the existing binary was built with db-surreal-rocksdb"
    fi

    DAEMON_CMD="./target/release/rust-daq-daemon daemon --port ${DAEMON_PORT}"

    if [[ -n "$RUNTIME_MODE" ]]; then
        # Validate against known modes to prevent shell injection via SSH
        case "$RUNTIME_MODE" in
            mock|native|universal|hybrid-db) ;;
            *) fail "Invalid --runtime-mode '${RUNTIME_MODE}'. Allowed: mock, native, universal, hybrid-db" ;;
        esac
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode ${RUNTIME_MODE}"
    elif $WITH_DB; then
        # Default to hybrid-db when DB features are compiled in
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode hybrid-db"
    else
        # --no-db: binary lacks DB features, use universal TOML only
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode universal"
    fi

    if $WITH_DB; then
        DAEMON_CMD="${DAEMON_CMD} --db-path data/surrealdb-leabs"
        remote "mkdir -p ${REMOTE_DIR}/data" 2>/dev/null || true
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
# Phase 3.5: Build and serve WASM GUI (optional)
# ============================================================================
if $WASM_GUI && ! $GUI_ONLY; then
    step "Phase 3.5: Building WASM GUI on leabs-dev"

    # Ensure trunk is installed (prefer /usr/local/bin to survive ~/.cargo/bin cleanup)
    if ! remote "command -v trunk >/dev/null 2>&1"; then
        info "Installing trunk (WASM build tool) to /usr/local/bin..."
        remote "source \$HOME/.cargo/env && cargo install trunk --locked --root /usr/local" 2>&1 | while IFS= read -r line; do
            echo -e "    ${line}"
        done
        ok "trunk installed to /usr/local/bin"
    else
        ok "trunk already installed ($(remote 'which trunk'))"
    fi

    # Ensure wasm32 target is installed
    remote "source \$HOME/.cargo/env && rustup target add wasm32-unknown-unknown" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done

    info "Building WASM GUI (trunk build --release)..."
    remote "
        source \$HOME/.cargo/env && \
        cd ${REMOTE_DIR}/crates/ui && \
        trunk build --release
    " 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done
    ok "WASM GUI built"

    # Kill any existing web server on port 8080
    remote "fuser -k 8080/tcp" 2>/dev/null || true
    sleep 1

    # Serve the WASM GUI
    remote "cd ${REMOTE_DIR}/crates/ui/dist && nohup python3 -m http.server 8080 > /tmp/wasm-gui-server.log 2>&1 &"
    sleep 2

    if remote "fuser 8080/tcp" &>/dev/null; then
        ok "WASM GUI serving on http://${LEABS_HOST}:8080"
    else
        warn "Failed to start WASM GUI web server on port 8080"
    fi
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
