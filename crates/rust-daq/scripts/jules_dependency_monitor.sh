#!/bin/bash
# Jules Dependency Monitor - Tracks blocked agents and notifies when dependencies are ready
# Jules-17: Dependency Coordinator
# Usage: ./scripts/jules_dependency_monitor.sh [--once|--daemon]

set -euo pipefail

BEADS_DB="${BEADS_DB:-.beads/daq.db}"
MONITOR_INTERVAL=300  # 5 minutes
NOTIFICATION_FILE="docs/project_management/JULES_NOTIFICATIONS.md"

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Task dependency mapping
declare -A DEPENDENCIES=(
    # Phase 4 Scripting Layer (CRITICAL)
    ["bd-svlx"]="bd-hqy6"  # Jules-11 needs Jules-10
    ["bd-dxqi"]="bd-hqy6 bd-svlx"  # Jules-12 needs Jules-10 + Jules-11
    ["bd-ya3l"]="bd-hqy6"  # Jules-14 needs Jules-10
    ["bd-u7hu"]="bd-dxqi"  # Jules-13 needs Jules-12

    # Phase 3 Data Layer
    ["bd-vkp3"]="bd-rcxa"  # Jules-9 needs Jules-8
)

# Agent assignment mapping
declare -A AGENT_MAP=(
    ["bd-95pj"]="Jules-2"
    ["bd-l7vs"]="Jules-3/Jules-4"
    ["bd-e18h"]="Jules-5"
    ["bd-op7v"]="Jules-6"
    ["bd-9cz0"]="Jules-7"
    ["bd-rcxa"]="Jules-8"
    ["bd-vkp3"]="Jules-9"
    ["bd-hqy6"]="Jules-10"
    ["bd-svlx"]="Jules-11"
    ["bd-dxqi"]="Jules-12"
    ["bd-u7hu"]="Jules-13"
    ["bd-ya3l"]="Jules-14"
)

# Task titles for better readability
declare -A TASK_TITLES=(
    ["bd-hqy6"]="Define ScriptEngine Trait"
    ["bd-svlx"]="PyO3 Backend Implementation"
    ["bd-dxqi"]="V3 API Python Bindings"
    ["bd-ya3l"]="Rhai/Lua Alternative Backend"
    ["bd-u7hu"]="Hot-Swappable Logic"
    ["bd-rcxa"]="Arrow Batching in DataDistributor"
    ["bd-vkp3"]="HDF5 + Arrow Integration"
    ["bd-95pj"]="ESP300 V3 Migration"
    ["bd-l7vs"]="MaiTai + Newport V3 Migration"
    ["bd-e18h"]="PVCAM V3 Fix"
    ["bd-op7v"]="Standardize Measurement Enum"
    ["bd-9cz0"]="Fix Trait Signature Mismatches"
)

log() {
    echo -e "${BLUE}[$(date '+%Y-%m-%d %H:%M:%S')]${NC} $1"
}

log_success() {
    echo -e "${GREEN}✅ $1${NC}"
}

log_warning() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

log_error() {
    echo -e "${RED}❌ $1${NC}"
}

# Check if a task is closed
is_task_closed() {
    local task_id="$1"
    local status
    status=$(BEADS_DB="$BEADS_DB" bd show "$task_id" --json 2>/dev/null | jq -r '.status' || echo "unknown")
    [[ "$status" == "closed" ]]
}

# Check if all dependencies are satisfied
check_dependencies() {
    local task_id="$1"
    local deps="${DEPENDENCIES[$task_id]:-}"

    if [[ -z "$deps" ]]; then
        return 0  # No dependencies
    fi

    for dep in $deps; do
        if ! is_task_closed "$dep"; then
            return 1  # Dependency not satisfied
        fi
    done

    return 0  # All dependencies satisfied
}

# Get list of currently blocked tasks
get_blocked_tasks() {
    local blocked=()
    for task_id in "${!DEPENDENCIES[@]}"; do
        if ! is_task_closed "$task_id" && ! check_dependencies "$task_id"; then
            blocked+=("$task_id")
        fi
    done
    echo "${blocked[@]}"
}

# Get list of newly ready tasks
get_ready_tasks() {
    local ready=()
    for task_id in "${!DEPENDENCIES[@]}"; do
        if ! is_task_closed "$task_id" && check_dependencies "$task_id"; then
            ready+=("$task_id")
        fi
    done
    echo "${ready[@]}"
}

# Send notification (append to notification file)
send_notification() {
    local task_id="$1"
    local agent="${AGENT_MAP[$task_id]:-Unknown}"
    local title="${TASK_TITLES[$task_id]:-$task_id}"
    local timestamp
    timestamp=$(date '+%Y-%m-%d %H:%M:%S')

    log_success "Task $task_id ($agent) is now READY: $title"

    # Append to notification file
    cat >> "$NOTIFICATION_FILE" <<EOF

## [$timestamp] Task Ready: $task_id

**Agent**: $agent
**Title**: $title
**Dependencies Satisfied**: ✅ All blockers completed

**Action Required**:
1. Create Jules session for this task
2. Provide reference implementations and architectural context
3. Monitor session progress
4. Update JULES_DEPENDENCY_MAP.md when complete

**Reference**:
- Dependency Map: docs/project_management/JULES_DEPENDENCY_MAP.md
- Task Details: bd show $task_id

---
EOF

    echo "Notification written to $NOTIFICATION_FILE"
}

# Monitor for newly ready tasks
monitor_dependencies() {
    log "Starting dependency monitoring..."
    log "Beads DB: $BEADS_DB"
    echo ""

    # Check for blocked tasks
    local blocked_tasks
    blocked_tasks=$(get_blocked_tasks)

    if [[ -n "$blocked_tasks" ]]; then
        log_warning "Currently blocked tasks:"
        for task_id in $blocked_tasks; do
            local agent="${AGENT_MAP[$task_id]:-Unknown}"
            local title="${TASK_TITLES[$task_id]:-$task_id}"
            local deps="${DEPENDENCIES[$task_id]:-}"

            echo "  - $task_id ($agent): $title"
            echo "    Waiting on: $deps"
        done
        echo ""
    fi

    # Check for ready tasks
    local ready_tasks
    ready_tasks=$(get_ready_tasks)

    if [[ -n "$ready_tasks" ]]; then
        log_success "Tasks ready to start:"
        for task_id in $ready_tasks; do
            local agent="${AGENT_MAP[$task_id]:-Unknown}"
            local title="${TASK_TITLES[$task_id]:-$task_id}"

            echo "  - $task_id ($agent): $title ✅"

            # Send notification if not already notified
            if ! grep -q "Task Ready: $task_id" "$NOTIFICATION_FILE" 2>/dev/null; then
                send_notification "$task_id"
            fi
        done
        echo ""
    fi

    # Progress report
    local total_tasks=${#DEPENDENCIES[@]}
    local ready_count=0
    local blocked_count=0
    local completed_count=0

    for task_id in "${!DEPENDENCIES[@]}"; do
        if is_task_closed "$task_id"; then
            ((completed_count++))
        elif check_dependencies "$task_id"; then
            ((ready_count++))
        else
            ((blocked_count++))
        fi
    done

    log "Progress: $completed_count completed, $ready_count ready, $blocked_count blocked (of $total_tasks tracked tasks)"

    # Critical path check
    if ! is_task_closed "bd-hqy6"; then
        log_error "CRITICAL: Jules-10 (bd-hqy6) ScriptEngine trait NOT STARTED - blocks 4 tasks"
    elif ! is_task_closed "bd-svlx"; then
        log_warning "Jules-11 (bd-svlx) PyO3 backend in progress - blocks 2 tasks"
    fi
}

# Daemon mode - continuous monitoring
daemon_mode() {
    log "Starting daemon mode (check interval: ${MONITOR_INTERVAL}s)"
    log "Press Ctrl+C to stop"
    echo ""

    # Initialize notification file
    if [[ ! -f "$NOTIFICATION_FILE" ]]; then
        cat > "$NOTIFICATION_FILE" <<EOF
# Jules Agent Dependency Notifications

**Jules-17 Dependency Coordinator - Automatic Notifications**

This file contains real-time notifications when Jules agent dependencies are satisfied.

**Legend**:
- ✅ Ready: All dependencies completed, task can start
- ⚠️ Blocked: Waiting on one or more dependencies
- 🔴 Critical: High-priority blocker affecting multiple tasks

---
EOF
    fi

    while true; do
        monitor_dependencies
        log "Next check in ${MONITOR_INTERVAL}s..."
        echo ""
        sleep "$MONITOR_INTERVAL"
    done
}

# One-time check
once_mode() {
    monitor_dependencies
}

# Main entry point
main() {
    local mode="${1:-once}"

    case "$mode" in
        --daemon|-d)
            daemon_mode
            ;;
        --once|-o)
            once_mode
            ;;
        *)
            log "Usage: $0 [--once|--daemon]"
            log "  --once   : Run dependency check once (default)"
            log "  --daemon : Continuous monitoring mode"
            exit 1
            ;;
    esac
}

main "$@"
