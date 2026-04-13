#!/usr/bin/env bash
# deploy.sh — Unified deploy script for all rust-daq targets
#
# Replaces deploy-maitai.sh and deploy-leabs.sh with a single entry point.
# Target profiles encapsulate SSH user, remote dir, env file, features, etc.
#
# Usage:
#   bash scripts/deploy/deploy.sh --target maitai                    # Full deploy to maitai
#   bash scripts/deploy/deploy.sh --target leabs-dev --wasm-gui      # Deploy to leabs with WASM GUI
#   bash scripts/deploy/deploy.sh --target maitai --branch feat/foo  # Feature branch deploy
#   bash scripts/deploy/deploy.sh --target leabs-dev --gui-only      # Just launch GUI
#
# See --help for all options.

set -euo pipefail

# ============================================================================
# Source shared deploy library
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPS_DIR="$(cd "${SCRIPT_DIR}/../ops" && pwd)"
# shellcheck source=deploy-common.sh
source "${SCRIPT_DIR}/deploy-common.sh"

# ============================================================================
# Target profiles
# ============================================================================
# Each target defines: SSH user, host, remote dir, env file, hardware config,
# default features, git remote name, cargo clean behavior, default DB state,
# and SDK check command.
#
# To add a new target, add a setup_<target>() function below.

setup_maitai() {
    TARGET_LABEL="maitai"
    TARGET_USER="${MAITAI_USER:-maitai}"
    TARGET_HOST="${MAITAI_HOST:-maitai-eos}"
    DEPLOY_SSH="${MAITAI_SSH:-${TARGET_USER}@${TARGET_HOST}}"
    REMOTE_DIR="${REMOTE_DIR:-/home/${TARGET_USER}/code/rust-daq}"
    ENV_FILE="config/hosts/maitai.env"
    HARDWARE_CONFIG=""  # maitai uses --runtime-mode, not --hardware-config
    DEFAULT_FEATURES="maitai,db"
    GIT_REMOTE="origin"
    CARGO_CLEAN=true
    DEFAULT_WITH_DB=false
    DEFAULT_RUNTIME_MODE="hybrid-db"
    BANNER_NAME="Maitai"
    DB_DATA_DIR="data/sqlite-maitai"
}

setup_leabs_dev() {
    TARGET_LABEL="leabs-dev"
    TARGET_USER="${LEABS_USER:-brian}"
    TARGET_HOST="${LEABS_HOST:-leabs-dev}"
    DEPLOY_SSH="${LEABS_SSH:-${TARGET_USER}@${TARGET_HOST}}"
    REMOTE_DIR="${REMOTE_DIR:-/home/${TARGET_USER}/code/rust-daq}"
    ENV_FILE="config/hosts/leabs-dev.env"
    HARDWARE_CONFIG="config/leabs_hardware.toml"
    DEFAULT_FEATURES="leabs_hardware,db"
    GIT_REMOTE="github"
    CARGO_CLEAN=false
    DEFAULT_WITH_DB=true
    DEFAULT_RUNTIME_MODE=""  # leabs uses --hardware-config, not --runtime-mode
    BANNER_NAME="LEABS"
    DB_DATA_DIR="data/sqlite-leabs"
}

# ============================================================================
# Defaults
# ============================================================================
TARGET=""
BRANCH="main"
WITH_DB=""  # empty = use target default
NO_DB=false
SKIP_BUILD=false
SKIP_GUI=false
GUI_ONLY=false
WASM_GUI=false
RUNTIME_MODE=""
DAEMON_PORT=50051
REMOTE_LOG="/tmp/rust-daq-daemon.log"

# ============================================================================
# Parse arguments
# ============================================================================
print_help() {
    cat <<'HELP'
deploy.sh — Unified deploy for rust-daq targets

USAGE:
  bash scripts/deploy/deploy.sh --target <name> [OPTIONS]

TARGETS:
  maitai                PVCAM + Comedi lab machine (maitai-eos)
  leabs-dev             Andor iStar + IPG laser VM (leabs-dev)

OPTIONS:
  --target <name>       Deploy target (required unless auto-detected)
  --branch <name>       Branch to checkout on remote (default: main)
  --with-db             Enable SQLite database persistence
  --no-db               Disable database persistence (override target default)
  --skip-build          Skip remote build (just restart daemon + launch GUI)
  --skip-gui            Don't launch local GUI (deploy daemon only)
  --daemon-only         Alias for --skip-gui
  --gui-only            Skip all remote steps, just launch local GUI
  --wasm-gui            Build and serve WASM GUI on remote (port 8080)
  --runtime-mode <mode> Override daemon runtime mode (mock|native|universal|hybrid-db)
  --help                Show this help

EXAMPLES:
  # Full deploy to maitai
  bash scripts/deploy/deploy.sh --target maitai

  # Deploy feature branch to leabs-dev with WASM GUI
  bash scripts/deploy/deploy.sh --target leabs-dev --branch feat/andor --wasm-gui

  # Just restart daemon on maitai (no build, no GUI)
  bash scripts/deploy/deploy.sh --target maitai --skip-build --daemon-only

  # Just launch GUI (daemon already running)
  bash scripts/deploy/deploy.sh --target leabs-dev --gui-only
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET="$2"
            shift 2
            ;;
        --branch)
            BRANCH="$2"
            shift 2
            ;;
        --with-db)
            WITH_DB=true
            shift
            ;;
        --no-db)
            NO_DB=true
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
# Resolve target
# ============================================================================
if [[ -z "$TARGET" ]]; then
    echo -e "${RED}Error: --target is required${NC}"
    echo ""
    print_help
    exit 1
fi

# Load target profile
case "$TARGET" in
    maitai)
        setup_maitai
        ;;
    leabs-dev|leabs)
        setup_leabs_dev
        ;;
    *)
        echo -e "${RED}Unknown target: ${TARGET}${NC}"
        echo "Available targets: maitai, leabs-dev"
        exit 1
        ;;
esac

# Resolve DB flag: explicit flag > target default
if $NO_DB; then
    WITH_DB=false
elif [[ -z "$WITH_DB" ]]; then
    WITH_DB=$DEFAULT_WITH_DB
fi

# ============================================================================
# Resolve cargo features
# ============================================================================
# Start with target defaults; adjust based on DB flag and SDK detection
CARGO_FEATURES="$DEFAULT_FEATURES"
if ! $WITH_DB; then
    # Strip db from features
    CARGO_FEATURES=$(echo "$CARGO_FEATURES" | sed 's/,db//' | sed 's/db,//' | sed 's/^db$//')
fi

# ============================================================================
# Banner
# ============================================================================
echo -e "${BOLD}${CYAN}"
echo "╔══════════════════════════════════════════════════╗"
printf "║        rust-daq %-16s Deploy           ║\n" "$BANNER_NAME"
echo "╚══════════════════════════════════════════════════╝"
echo -e "${NC}"
echo -e "  Target:     ${BOLD}${DEPLOY_SSH}${NC}"
echo -e "  Branch:     ${BOLD}${BRANCH}${NC}"
echo -e "  Features:   ${BOLD}${CARGO_FEATURES}${NC}"
echo -e "  Database:   ${BOLD}$(${WITH_DB} && echo 'enabled (SQLite)' || echo 'disabled')${NC}"
echo -e "  Build:      ${BOLD}$(${SKIP_BUILD} && echo 'skip' || echo 'release')${NC}"
echo -e "  GUI:        ${BOLD}$(${SKIP_GUI} && echo 'skip' || echo 'launch locally')${NC}"
if [[ -n "$RUNTIME_MODE" ]]; then
    echo -e "  Mode:       ${BOLD}${RUNTIME_MODE}${NC}"
fi
if $WASM_GUI; then
    echo -e "  WASM GUI:   ${BOLD}enabled${NC}"
fi
echo ""

# ============================================================================
# Phase 0: Connectivity & prerequisites check
# ============================================================================
if ! $GUI_ONLY; then
    deploy_check_ssh "$TARGET_LABEL"
    deploy_check_rust

    # Run SDK detection on remote to validate hardware availability
    step "SDK detection"
    SDK_OUTPUT=$(remote "bash ${REMOTE_DIR}/scripts/ops/detect-sdk.sh 2>&1 1>/dev/null" 2>/dev/null || true)
    if [[ -n "$SDK_OUTPUT" ]]; then
        echo "$SDK_OUTPUT" | while IFS= read -r line; do
            echo -e "  ${line}"
        done
    fi
fi

# ============================================================================
# Phase 1: Remote pull & build
# ============================================================================
if ! $GUI_ONLY && ! $SKIP_BUILD; then
    step "Phase 1: Pull & build on ${TARGET_LABEL} (branch: ${BRANCH})"

    deploy_fetch_and_checkout "$BRANCH" "$GIT_REMOTE"

    # Verify SDK environment via env file
    info "Verifying SDK environment via ${ENV_FILE}..."
    SDK_CHECK=$(remote "source ${REMOTE_DIR}/${ENV_FILE} && env" 2>/dev/null || true)
    if echo "$SDK_CHECK" | grep -q 'PVCAM_SDK_DIR\|ANDOR_SDK3_DIR'; then
        ok "SDK environment variables present"
    else
        warn "No SDK environment variables detected in ${ENV_FILE}"
    fi

    info "Building (release, ${CARGO_FEATURES})..."
    info "This will take several minutes..."

    # Build command: optionally cargo clean, then build
    BUILD_CMD="source \$HOME/.cargo/env 2>/dev/null || true && cd ${REMOTE_DIR} && source ${ENV_FILE}"
    if $CARGO_CLEAN; then
        BUILD_CMD="${BUILD_CMD} && cargo clean 2>/dev/null || true"
    fi
    BUILD_CMD="${BUILD_CMD} && cargo build --release -p bin --features ${CARGO_FEATURES}"

    remote "${BUILD_CMD}" 2>&1 | while IFS= read -r line; do
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
        warn "--skip-build with --with-db: ensure the existing binary was built with the db feature"
    fi

    # Build daemon command line
    DAEMON_CMD="./target/release/rust-daq-daemon daemon --port ${DAEMON_PORT}"

    if [[ -n "$RUNTIME_MODE" ]]; then
        deploy_validate_runtime_mode "$RUNTIME_MODE"
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode ${RUNTIME_MODE}"
    elif [[ -n "$DEFAULT_RUNTIME_MODE" ]]; then
        # Target has a default runtime mode (e.g., maitai -> hybrid-db)
        DAEMON_CMD="${DAEMON_CMD} --runtime-mode ${DEFAULT_RUNTIME_MODE}"
    elif [[ -n "$HARDWARE_CONFIG" ]]; then
        # Target uses hardware config file (e.g., leabs-dev)
        DAEMON_CMD="${DAEMON_CMD} --hardware-config ${HARDWARE_CONFIG}"
    fi

    if $WITH_DB; then
        DAEMON_CMD="${DAEMON_CMD} --db-path ${DB_DATA_DIR}"
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
    step "Phase 3.5: Building WASM GUI on ${TARGET_LABEL}"

    # Ensure trunk is installed (pre-built binary — fast, no cargo build)
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
        ok "WASM GUI serving on http://${TARGET_HOST}:8080"
    else
        warn "Failed to start WASM GUI web server on port 8080"
    fi
fi

# ============================================================================
# Phase 4: Launch local GUI
# ============================================================================
if ! $SKIP_GUI; then
    deploy_launch_gui "$TARGET_HOST" "deploy.sh --target ${TARGET}"
fi

# ============================================================================
# Done
# ============================================================================
echo ""
echo -e "${GREEN}${BOLD}Deploy complete.${NC}"
