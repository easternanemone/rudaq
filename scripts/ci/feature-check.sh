#!/usr/bin/env bash
# feature-check.sh — Run cargo-hack feature powerset check on key crates
#
# Usage:
#   bash scripts/ci/feature-check.sh              # All crates
#   bash scripts/ci/feature-check.sh common       # Single crate
#   bash scripts/ci/feature-check.sh --quick      # Fast: each-feature (not powerset)

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m'

MODE="--feature-powerset"
CRATE=""

for arg in "$@"; do
    case "$arg" in
        --quick) MODE="--each-feature" ;;
        *) CRATE="$arg" ;;
    esac
done

# FFI features that need SDK headers (skip on macOS dev machines)
REGISTRY_SKIP="comedi,comedi_hardware,pvcam,pvcam_sdk,pvcam_hardware,andor,andor_hardware,all_hardware,full"

check_crate() {
    local pkg="$1"
    local skip="${2:-}"
    local skip_arg=""
    if [ -n "$skip" ]; then
        skip_arg="--skip $skip"
    fi

    echo -en "  ${BOLD}$pkg${NC} ($MODE)...  "
    if cargo hack check -p "$pkg" $MODE --no-dev-deps $skip_arg 2>/dev/null; then
        echo -e "${GREEN}ok${NC}"
    else
        echo -e "${RED}FAILED${NC}"
        echo -e "  Rerun with output: cargo hack check -p $pkg $MODE --no-dev-deps $skip_arg"
        return 1
    fi
}

echo -e "${BOLD}Feature powerset check${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

failed=0

if [ -n "$CRATE" ]; then
    skip=""
    [ "$CRATE" = "driver-registry" ] && skip="$REGISTRY_SKIP"
    [ "$CRATE" = "storage" ] && skip="storage_hdf5"
    check_crate "$CRATE" "$skip" || failed=1
else
    check_crate "common" "" || failed=1
    check_crate "pool" "" || failed=1
    check_crate "experiment" "" || failed=1
    check_crate "storage" "storage_hdf5" || failed=1
    check_crate "driver-registry" "$REGISTRY_SKIP" || failed=1
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [ $failed -ne 0 ]; then
    echo -e "${RED}Feature check failed.${NC}"
    exit 1
fi
echo -e "${GREEN}All feature combinations passed.${NC}"
