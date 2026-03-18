#!/usr/bin/env bash
# pre-push-gate.sh — Quality gate that mirrors CI checks
#
# Runs: cargo fmt --check, cargo clippy -D warnings, cargo nextest run
# Called by the composite pre-push hook in .beads/hooks/pre-push
#
# Exit code 0 = all checks passed, nonzero = push blocked.

set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

failed=0

echo -e "${BOLD}Pre-push quality gate${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── 1. Format check ──────────────────────────────────────────────
echo -en "  Checking format...  "
if cargo fmt --all -- --check >/dev/null 2>&1; then
    echo -e "${GREEN}ok${NC}"
else
    echo -e "${RED}FAILED${NC}"
    echo -e "  ${YELLOW}Fix: cargo fmt --all${NC}"
    failed=1
fi

# ── 1b. mdBook docs build ────────────────────────────────────────
if command -v mdbook >/dev/null 2>&1; then
    echo -en "  Building docs...    "
    if mdbook build docs/ 2>&1 | grep -q "^ERROR"; then
        echo -e "${RED}FAILED${NC}"
        echo -e "  ${YELLOW}Fix: mdbook build docs/${NC}"
        failed=1
    else
        echo -e "${GREEN}ok${NC}"
    fi
else
    echo -e "  Building docs...    ${YELLOW}skip${NC} (mdbook not installed)"
fi

# ── 2. Clippy ────────────────────────────────────────────────────
echo -en "  Running clippy...   "
# Match CI: workspace scope, exclude ui + comedi crates, deny warnings
if cargo clippy --workspace --all-targets \
    --exclude ui \
    --exclude comedi-sys \
    --exclude driver-comedi \
    -- -D warnings >/dev/null 2>&1; then
    echo -e "${GREEN}ok${NC}"
else
    echo -e "${RED}FAILED${NC}"
    echo -e "  ${YELLOW}Run: cargo clippy --workspace --all-targets -- -D warnings${NC}"
    failed=1
fi

# ── 3. Tests ─────────────────────────────────────────────────────
echo -en "  Running tests...    "
if command -v cargo-nextest >/dev/null 2>&1; then
    if cargo nextest run --workspace --exclude ui --color=never >/dev/null 2>&1; then
        echo -e "${GREEN}ok${NC}"
    else
        echo -e "${RED}FAILED${NC}"
        echo -e "  ${YELLOW}Run: cargo nextest run --workspace --exclude ui${NC}"
        failed=1
    fi
else
    if cargo test --workspace --exclude ui >/dev/null 2>&1; then
        echo -e "${GREEN}ok${NC}"
    else
        echo -e "${RED}FAILED${NC}"
        echo -e "  ${YELLOW}Run: cargo test --workspace --exclude ui${NC}"
        failed=1
    fi
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ $failed -ne 0 ]; then
    echo -e "${RED}Quality gate failed — push blocked.${NC}"
    echo -e "Skip with: git push --no-verify ${YELLOW}(not recommended)${NC}"
    exit 1
fi

echo -e "${GREEN}All checks passed.${NC}"
exit 0
