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
#   bash scripts/deploy/deploy-leabs.sh                           # Full deploy from main
#   bash scripts/deploy/deploy-leabs.sh --branch feat/my-feature  # Deploy a feature branch
#   bash scripts/deploy/deploy-leabs.sh --gui-only                # Just launch GUI (daemon running)
#   bash scripts/deploy/deploy-leabs.sh --skip-build --daemon-only  # Restart daemon, skip build
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
LEABS_USER="${LEABS_USER:-brian}"
LEABS_HOST="${LEABS_HOST:-leabs-dev}"  # Tailscale hostname
LEABS_SSH="${LEABS_SSH:-${LEABS_USER}@${LEABS_HOST}}"
DEPLOY_SSH="$LEABS_SSH"
REMOTE_DIR="${REMOTE_DIR:-/home/${LEABS_USER}/code/rust-daq}"
DAEMON_PORT=50051
REMOTE_LOG="/tmp/rust-daq-daemon.log"
ENV_FILE="config/hosts/leabs-dev.env"
HARDWARE_CONFIG="config/leabs_hardware.toml"
CARGO_FEATURES="leabs_hardware,db-surreal-rocksdb"

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
  bash scripts/deploy/deploy-leabs.sh

  # Deploy a feature branch
  bash scripts/deploy/deploy-leabs.sh --branch feat/leabs-andor-hardware

  # Just restart daemon (no build, no GUI)
  bash scripts/deploy/deploy-leabs.sh --skip-build --daemon-only

  # Just launch GUI (daemon already running on leabs-dev)
  bash scripts/deploy/deploy-leabs.sh --gui-only
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
    deploy_check_ssh "leabs-dev"
    deploy_check_rust

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

    deploy_fetch_and_checkout "$BRANCH" "github"

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

    DAEMON_CMD="./target/release/rust-daq-daemon daemon --port ${DAEMON_PORT}"

    if [[ -n "$RUNTIME_MODE" ]]; then
        deploy_validate_runtime_mode "$RUNTIME_MODE"
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode ${RUNTIME_MODE}"
    else
        # Default to hardware-config for device registration. --runtime-mode and
        # --hardware-config are mutually exclusive in the CLI, so only add
        # --runtime-mode when the caller explicitly requests it.
        DAEMON_CMD="${DAEMON_CMD} --hardware-config ${HARDWARE_CONFIG}"
    fi

    if $WITH_DB; then
        # SurrealDB persistence works alongside --hardware-config via --db-path.
        DAEMON_CMD="${DAEMON_CMD} --db-path data/surrealdb-leabs"
        remote "mkdir -p ${REMOTE_DIR}/data" 2>/dev/null || true
    fi

    deploy_start_daemon "$DAEMON_CMD" "$ENV_FILE"
    deploy_wait_for_daemon
    deploy_show_startup_log
fi

# ============================================================================
# Phase 3.5: Build and serve WASM GUI (optional)
# ============================================================================
if $WASM_GUI && ! $GUI_ONLY; then
    step "Phase 3.5: Building WASM GUI on leabs-dev"

    # Ensure trunk is installed (pre-built binary in /usr/local/bin — fast, no cargo build)
    TRUNK_VERSION="0.21.14"
    if ! remote "command -v trunk >/dev/null 2>&1"; then
        info "Installing trunk ${TRUNK_VERSION} (pre-built binary)..."
        remote "curl -sL https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-x86_64-unknown-linux-gnu.tar.gz | sudo tar xz -C /usr/local/bin/"
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
    deploy_launch_gui "$LEABS_SSH" "deploy-leabs.sh"
fi

# ============================================================================
# Done
# ============================================================================
echo ""
echo -e "${GREEN}${BOLD}Deploy complete.${NC}"
