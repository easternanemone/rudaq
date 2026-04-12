#!/bin/bash
# runner-health-check.sh — Check self-hosted GitHub Actions runner health
#
# Usage: bash scripts/ops/runner-health-check.sh [--fix]
#
# Checks:
#   1. Both runners registered and online via GitHub API
#   2. SSH reachable
#   3. Runner process alive
#   4. Disk space adequate (>8GB free)
#   5. Memory not exhausted
#   6. Orphaned ssh-agent count
#   7. Stuck/queued jobs
#
# With --fix: kills orphan ssh-agents, restarts unhealthy runners

set -euo pipefail

FIX=false
[[ "${1:-}" == "--fix" ]] && FIX=true

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

ERRORS=0

ok()   { echo -e "  ${GREEN}[OK]${NC} $1"; }
warn() { echo -e "  ${YELLOW}[WARN]${NC} $1"; }
fail() { echo -e "  ${RED}[FAIL]${NC} $1"; ERRORS=$((ERRORS + 1)); }

echo "=== GitHub Actions Runner Health Check ==="
echo "  Date: $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
echo ""

# --- 1. GitHub API status ---
echo "--- GitHub API Runner Status ---"
RUNNERS_JSON=$(gh api repos/TheFermiSea/rust-daq/actions/runners 2>/dev/null || echo '{}')
RUNNER_COUNT=$(echo "$RUNNERS_JSON" | jq -r '.total_count // 0')

if [[ "$RUNNER_COUNT" -eq 0 ]]; then
    fail "No runners found via GitHub API"
else
    echo "$RUNNERS_JSON" | jq -r '.runners[] | "\(.name)\t\(.status)\t\(.busy)"' | while IFS=$'\t' read -r name status busy; do
        if [[ "$status" == "online" ]]; then
            ok "$name: online (busy=$busy)"
        else
            fail "$name: $status"
        fi
    done
fi
echo ""

# --- 2-6. Per-machine checks ---
declare -A MACHINES=(
    ["leabs-dev"]="leabs-dev"
    ["maitai-eos"]="maitai@100.117.5.12"
)
declare -A SERVICE_NAMES=(
    ["leabs-dev"]="actions.runner.TheFermiSea-rust-daq.leabs-dev.service"
    ["maitai-eos"]="actions.runner.TheFermiSea-rust-daq.maitai-eos.service"
)
declare -A RUNNER_DIRS=(
    ["leabs-dev"]="/home/brian/actions-runner-rust-daq"
    ["maitai-eos"]="/home/maitai/actions-runner-rust-daq"
)
declare -A RUNNER_USERS=(
    ["leabs-dev"]="brian"
    ["maitai-eos"]="maitai"
)

for machine in leabs-dev maitai-eos; do
    echo "--- $machine ---"
    ssh_target="${MACHINES[$machine]}"
    service="${SERVICE_NAMES[$machine]}"
    runner_dir="${RUNNER_DIRS[$machine]}"
    runner_user="${RUNNER_USERS[$machine]}"

    # SSH connectivity
    if ! ssh -o ConnectTimeout=5 -o BatchMode=yes "$ssh_target" "true" 2>/dev/null; then
        fail "$machine: SSH unreachable"
        echo ""
        continue
    fi
    ok "$machine: SSH reachable"

    # Runner process
    runner_pid=$(ssh -o ConnectTimeout=5 "$ssh_target" "pgrep -f 'Runner.Listener' 2>/dev/null || true" 2>/dev/null)
    if [[ -n "$runner_pid" ]]; then
        ok "Runner.Listener running (PID $runner_pid)"
    else
        fail "Runner.Listener not running"
        if $FIX; then
            echo "  Attempting restart..."
            ssh -o ConnectTimeout=5 "$ssh_target" "sudo systemctl restart $service" 2>/dev/null && ok "Service restarted" || fail "Restart failed"
        fi
    fi

    # Disk space
    avail_gb=$(ssh -o ConnectTimeout=5 "$ssh_target" "df --output=avail / | tail -1" 2>/dev/null | tr -d ' ')
    avail_gb=$((avail_gb / 1024 / 1024))
    if [[ "$avail_gb" -lt 8 ]]; then
        fail "Disk: ${avail_gb}GB free (need >8GB)"
    elif [[ "$avail_gb" -lt 15 ]]; then
        warn "Disk: ${avail_gb}GB free (low)"
    else
        ok "Disk: ${avail_gb}GB free"
    fi

    # Memory
    mem_info=$(ssh -o ConnectTimeout=5 "$ssh_target" "free -m | grep Mem" 2>/dev/null)
    mem_total=$(echo "$mem_info" | awk '{print $2}')
    mem_avail=$(echo "$mem_info" | awk '{print $7}')
    mem_pct=$((100 * mem_avail / mem_total))
    if [[ "$mem_pct" -lt 10 ]]; then
        fail "Memory: ${mem_avail}MB available of ${mem_total}MB (${mem_pct}% free)"
    elif [[ "$mem_pct" -lt 25 ]]; then
        warn "Memory: ${mem_avail}MB available of ${mem_total}MB (${mem_pct}% free)"
    else
        ok "Memory: ${mem_avail}MB available of ${mem_total}MB (${mem_pct}% free)"
    fi

    # Orphaned ssh-agents
    agent_count=$(ssh -o ConnectTimeout=5 "$ssh_target" "pgrep -u $runner_user ssh-agent 2>/dev/null | wc -l" 2>/dev/null || echo "0")
    if [[ "$agent_count" -gt 5 ]]; then
        warn "Orphaned ssh-agents: $agent_count"
        if $FIX; then
            ssh -o ConnectTimeout=5 "$ssh_target" "pkill -u $runner_user ssh-agent 2>/dev/null || true" 2>/dev/null
            ok "Killed orphaned ssh-agents"
        fi
    else
        ok "ssh-agents: $agent_count (healthy)"
    fi

    # Runner error rate (last log file)
    err_count=$(ssh -o ConnectTimeout=5 "$ssh_target" "grep -c 'ERR' \$(ls -t ${runner_dir}/_diag/Runner_*.log 2>/dev/null | head -1) 2>/dev/null || echo 0" 2>/dev/null)
    if [[ "$err_count" -gt 500 ]]; then
        warn "Runner log errors: $err_count (consider restart)"
    else
        ok "Runner log errors: $err_count"
    fi

    # Service uptime
    uptime_info=$(ssh -o ConnectTimeout=5 "$ssh_target" "systemctl show $service --property=ActiveEnterTimestamp 2>/dev/null" 2>/dev/null | cut -d= -f2)
    if [[ -n "$uptime_info" ]]; then
        ok "Service started: $uptime_info"
    fi

    echo ""
done

# --- 7. Queued jobs ---
echo "--- Queued Jobs ---"
queued=$(gh run list --status queued --json databaseId,displayTitle,createdAt,workflowName 2>/dev/null || echo '[]')
queued_count=$(echo "$queued" | jq 'length')
if [[ "$queued_count" -gt 0 ]]; then
    echo "$queued" | jq -r '.[] | "  \(.workflowName): \(.displayTitle) (queued since \(.createdAt))"'

    # Check for jobs queued > 30 minutes
    now=$(date +%s)
    echo "$queued" | jq -r '.[].createdAt' | while read -r ts; do
        created=$(date -d "$ts" +%s 2>/dev/null || date -j -f "%Y-%m-%dT%H:%M:%SZ" "$ts" +%s 2>/dev/null || echo 0)
        age_min=$(( (now - created) / 60 ))
        if [[ "$age_min" -gt 30 ]]; then
            warn "Job queued for ${age_min} minutes (possible stuck runner)"
        fi
    done
else
    ok "No queued jobs"
fi
echo ""

# --- Summary ---
if [[ "$ERRORS" -gt 0 ]]; then
    echo -e "${RED}Health check completed with $ERRORS error(s)${NC}"
    exit 1
else
    echo -e "${GREEN}All checks passed${NC}"
fi
