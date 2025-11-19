#!/bin/bash
# Monitor running tests on maitai-eos
#
# Usage: ./scripts/remote/monitor_tests.sh [OPTIONS]
#
# This script watches test execution in real-time, showing:
#   - Current status
#   - Progress
#   - Any failures
#   - Estimated time remaining

set -e

# Configuration
REMOTE_HOST="maitai-eos"
REMOTE_DIR="~/rust-daq"
REMOTE_LOG="$REMOTE_DIR/test_output.log"
UPDATE_INTERVAL=5

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# State tracking
LAST_LINE_COUNT=0
START_TIME=$(date +%s)
TESTS_TOTAL=0
TESTS_PASSED=0
TESTS_FAILED=0

# Helper functions
header() {
    clear
    echo -e "${CYAN}╔════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║  Test Monitor - $REMOTE_HOST${NC}"
    echo -e "${CYAN}╚════════════════════════════════════════════════════╝${NC}"
    echo ""
}

log() {
    echo -e "${BLUE}►${NC} $1"
}

success() {
    echo -e "${GREEN}✓${NC} $1"
}

error() {
    echo -e "${RED}✗${NC} $1"
}

warning() {
    echo -e "${YELLOW}!${NC} $1"
}

show_progress() {
    local elapsed=$(($(date +%s) - START_TIME))
    local mins=$((elapsed / 60))
    local secs=$((elapsed % 60))

    echo ""
    echo -e "${CYAN}Progress:${NC}"
    echo -e "  Elapsed: ${YELLOW}${mins}m ${secs}s${NC}"

    if [ $TESTS_TOTAL -gt 0 ]; then
        local percent=$((TESTS_PASSED * 100 / TESTS_TOTAL))
        local bar_filled=$((percent / 5))
        local bar_empty=$((20 - bar_filled))

        printf "  Progress: ["
        printf "%${bar_filled}s" | tr ' ' '='
        printf "%${bar_empty}s" | tr ' ' '-'
        printf "] %d%%\n" $percent

        echo -e "  Tests: ${GREEN}${TESTS_PASSED}${NC} passed, ${RED}${TESTS_FAILED}${NC} failed, ${YELLOW}${TESTS_TOTAL}${NC} total"
    fi
}

# Check if test is running
is_test_running() {
    if ssh -o ConnectTimeout=5 "$REMOTE_HOST" "pgrep -f 'cargo test' > /dev/null 2>&1"; then
        return 0
    else
        return 1
    fi
}

# Verify prerequisites
echo -e "${BLUE}Checking prerequisites...${NC}"

if ! ssh -o ConnectTimeout=5 "$REMOTE_HOST" 'echo "OK"' > /dev/null 2>&1; then
    error "Cannot connect to $REMOTE_HOST"
    exit 1
fi
success "SSH connection OK"

if ! ssh "$REMOTE_HOST" "test -f $REMOTE_LOG 2>/dev/null"; then
    warning "Test output log not found. Starting monitor..."
    log "Tests may be queued or starting up"
fi

# Main monitoring loop
header

echo -e "${BLUE}Waiting for tests to start on $REMOTE_HOST...${NC}"
echo "This window will update every ${UPDATE_INTERVAL}s"
echo ""
echo "Press Ctrl+C to stop monitoring"
echo ""

TESTS_STARTED=false

while true; do
    header

    # Check if tests are running
    if is_test_running; then
        TESTS_STARTED=true
        log "Tests are currently running"
    else
        if [ "$TESTS_STARTED" = true ]; then
            success "Tests completed (no longer running)"
        else
            log "Waiting for tests to start..."
        fi
    fi

    # Get current log content
    if ssh "$REMOTE_HOST" "test -f $REMOTE_LOG"; then
        CURRENT_LINES=$(ssh "$REMOTE_HOST" "wc -l < $REMOTE_LOG 2>/dev/null || echo 0")

        if [ "$CURRENT_LINES" -gt "$LAST_LINE_COUNT" ]; then
            log "Log file growing: $CURRENT_LINES lines"

            # Update test counts
            TESTS_TOTAL=$(ssh "$REMOTE_HOST" "grep -c '^test ' $REMOTE_LOG 2>/dev/null || echo 0")
            TESTS_PASSED=$(ssh "$REMOTE_HOST" "grep -c ' ok$' $REMOTE_LOG 2>/dev/null || echo 0")
            TESTS_FAILED=$(ssh "$REMOTE_HOST" "grep -c ' FAILED$' $REMOTE_LOG 2>/dev/null || echo 0")

            LAST_LINE_COUNT=$CURRENT_LINES

            # Show recently updated tests
            echo ""
            echo -e "${CYAN}Recently completed tests:${NC}"
            ssh "$REMOTE_HOST" "tail -10 $REMOTE_LOG" 2>/dev/null | sed 's/^/  /'
        fi
    fi

    # Show progress
    show_progress

    # Check for failures
    if [ "$TESTS_FAILED" -gt 0 ]; then
        echo ""
        echo -e "${RED}FAILURES DETECTED:${NC}"
        ssh "$REMOTE_HOST" "grep 'FAILED' $REMOTE_LOG 2>/dev/null || true" | head -5 | sed 's/^/  /'
    fi

    # Check cargo process
    CARGO_PROCESSES=$(ssh "$REMOTE_HOST" "pgrep -f 'cargo test' | wc -l" 2>/dev/null || echo 0)
    echo ""
    echo -e "${CYAN}Status:${NC}"
    if [ "$CARGO_PROCESSES" -gt 0 ]; then
        echo -e "  ${GREEN}Running${NC} ($CARGO_PROCESSES processes)"
    else
        echo -e "  ${YELLOW}Not running${NC}"
    fi

    # Get disk usage
    DISK_USAGE=$(ssh "$REMOTE_HOST" "du -sh $REMOTE_DIR 2>/dev/null | cut -f1" || echo "unknown")
    echo "  Disk: $DISK_USAGE"

    # Get memory usage
    MEM_USAGE=$(ssh "$REMOTE_HOST" "free -h 2>/dev/null | grep Mem | awk '{print \$3 \"/\" \$2}'" || echo "unknown")
    echo "  Memory: $MEM_USAGE"

    echo ""
    echo -e "${BLUE}Next update in ${UPDATE_INTERVAL}s... (Ctrl+C to exit)${NC}"

    sleep "$UPDATE_INTERVAL"
done
