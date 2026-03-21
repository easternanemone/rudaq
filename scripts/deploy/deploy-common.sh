#!/usr/bin/env bash
# deploy-common.sh — Shared functions for deploy-maitai.sh and deploy-leabs.sh
#
# Source this file from a deploy script after setting the required variables:
#   DEPLOY_SSH    — SSH target (user@host)
#   REMOTE_DIR    — Remote checkout path
#   DAEMON_PORT   — gRPC port (default 50051)
#   REMOTE_LOG    — Remote log path (default /tmp/rust-daq-daemon.log)
#
# Provides:
#   Color variables, step/ok/warn/fail/info helpers, remote() SSH wrapper,
#   deploy_check_ssh(), deploy_check_rust(), deploy_fetch_and_checkout(),
#   deploy_validate_branch(), deploy_stop_daemon(), deploy_build_daemon_cmd(),
#   deploy_start_daemon(), deploy_wait_for_daemon(), deploy_show_startup_log()

# ============================================================================
# Colors (idempotent — safe to source multiple times)
# ============================================================================
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ============================================================================
# Output helpers
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

# ============================================================================
# SSH wrapper
# ============================================================================
remote() {
    ssh -o ConnectTimeout=10 -o BatchMode=yes "${DEPLOY_SSH}" "$@"
}

# ============================================================================
# Phase 0 helpers: connectivity and toolchain
# ============================================================================
deploy_check_ssh() {
    local label="${1:-remote host}"
    step "Phase 0: Checking SSH connectivity to ${label}"
    if ! remote "echo ok" &>/dev/null; then
        fail "Cannot SSH to ${DEPLOY_SSH}. Is Tailscale running?"
    fi
    ok "SSH to ${DEPLOY_SSH} works"
}

deploy_check_rust() {
    # Check Rust toolchain — try PATH first, then source cargo env as fallback
    # (SSH BatchMode doesn't load login profile, and rustc may be installed
    # via system packages without $HOME/.cargo/env)
    if ! remote 'command -v rustc >/dev/null 2>&1 || { [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"; command -v rustc >/dev/null 2>&1; }' &>/dev/null; then
        warn "Rust toolchain not found on remote"
        info "Install with: ssh ${DEPLOY_SSH} 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh'"
        fail "Rust toolchain required for remote build"
    fi
    ok "Rust toolchain available"
}

# ============================================================================
# Phase 1 helpers: fetch, checkout, build
# ============================================================================
deploy_validate_branch() {
    local branch="$1"
    if [[ ! "$branch" =~ ^[a-zA-Z0-9._/-]+$ ]]; then
        fail "Invalid branch name '${branch}' — only alphanumeric, '.', '_', '/', '-' allowed"
    fi
}

deploy_fetch_and_checkout() {
    local branch="$1"
    local git_remote="${2:-origin}"  # allow overriding remote name (leabs uses "github")

    info "Fetching latest code..."
    remote "cd ${REMOTE_DIR} && git fetch --all --prune" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done

    deploy_validate_branch "$branch"

    info "Checking out ${branch}..."
    remote "cd ${REMOTE_DIR} && git checkout \"${branch}\" && git pull ${git_remote} \"${branch}\"" 2>&1 | while IFS= read -r line; do
        echo -e "    ${line}"
    done
    ok "On branch ${branch}, up to date"
}

deploy_verify_binary() {
    remote "test -f ${REMOTE_DIR}/target/release/rust-daq-daemon" || fail "Binary not found after build"
    ok "Binary verified: ${REMOTE_DIR}/target/release/rust-daq-daemon"
}

# ============================================================================
# Phase 2: Stop daemon
# ============================================================================
deploy_stop_daemon() {
    step "Phase 2: Stopping old daemon"

    local old_pids
    old_pids=$(remote "pgrep -f 'rust-daq-daemon daemon'" 2>/dev/null || true)
    if [[ -n "$old_pids" ]]; then
        local pid_list
        pid_list=$(echo "$old_pids" | tr '\n' ' ')
        info "Killing daemon process(es): ${pid_list}"
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
}

# ============================================================================
# Phase 3 helpers: build command, start, and wait
# ============================================================================

# Validate --runtime-mode value
deploy_validate_runtime_mode() {
    local mode="$1"
    case "$mode" in
        mock|native|universal|hybrid-db) ;;
        *) fail "Invalid --runtime-mode '${mode}'. Allowed: mock, native, universal, hybrid-db" ;;
    esac
}

deploy_start_daemon() {
    local daemon_cmd="$1"
    local env_file="$2"  # relative to REMOTE_DIR

    info "Command: ${daemon_cmd}"
    info "Log: ${REMOTE_LOG}"

    remote "
        source \$HOME/.cargo/env 2>/dev/null || true && \
        cd ${REMOTE_DIR} && \
        source ${env_file} && \
        nohup ${daemon_cmd} > ${REMOTE_LOG} 2>&1 &
        echo \$!
    "
}

deploy_wait_for_daemon() {
    info "Waiting for daemon to start (port ${DAEMON_PORT})..."
    local ready=false
    for i in $(seq 1 60); do
        if remote "ss -tlnp 2>/dev/null | grep -q ':${DAEMON_PORT}'" 2>/dev/null; then
            ready=true
            break
        fi
        sleep 1
        printf "."
    done
    echo ""

    if $ready; then
        ok "Daemon listening on port ${DAEMON_PORT}"
    else
        fail "Daemon failed to start within 60s. Check logs: ssh ${DEPLOY_SSH} 'tail -50 ${REMOTE_LOG}'"
    fi
}

deploy_show_startup_log() {
    info "Daemon startup log:"
    remote "head -20 ${REMOTE_LOG}" 2>/dev/null | while IFS= read -r line; do
        echo -e "    ${line}"
    done

    local new_pid
    new_pid=$(remote "pgrep -f 'rust-daq-daemon daemon'" 2>/dev/null || echo "unknown")
    ok "Daemon running (PID: ${new_pid})"
}

# ============================================================================
# Phase 4: GUI helpers
# ============================================================================
deploy_launch_gui() {
    local host="$1"
    local deploy_script_name="$2"

    step "Phase 4: Launching local GUI"
    info "Connecting to http://${host}:${DAEMON_PORT}"
    info "Close the GUI window or press Ctrl+C to exit"
    echo ""

    cargo run --bin rust-daq-gui -- --daemon-url "http://${host}:${DAEMON_PORT}" || true

    echo ""
    echo -e "${GREEN}GUI closed. Daemon still running on remote.${NC}"
    echo -e "  Daemon log:  ${BLUE}ssh ${DEPLOY_SSH} 'tail -f ${REMOTE_LOG}'${NC}"
    echo -e "  Stop daemon: ${BLUE}ssh ${DEPLOY_SSH} 'pkill -f rust-daq-daemon'${NC}"
    echo -e "  Reconnect:   ${BLUE}bash scripts/deploy/${deploy_script_name} --gui-only${NC}"
}
