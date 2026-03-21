#!/usr/bin/env bash
# pre-push-gate.sh — CANONICAL quality gate (mirrors CI)
#
# This is the ONE script that gates pushes. It mirrors the `validate` job
# in .github/workflows/ci.yml so that failures are caught locally before
# hitting CI.
#
# Checks (in order):
#   1. cargo fmt --check
#   2. mdBook docs build (if mdbook is installed)
#   3. cargo clippy (workspace, -D warnings, excluding ui/comedi)
#   4. cargo nextest run (workspace, excluding ui)
#
# Invocation paths:
#   - Automatic: .beads/hooks/pre-push calls this script
#   - Manual:    bash scripts/ci/pre-push-gate.sh
#
# Related but NOT overlapping:
#   - .pre-commit-config.yaml  — pre-COMMIT hooks (fmt, ast-grep, fast unit tests)
#   - .pre-commit-quick.yaml   — lightweight pre-commit (fmt + ast-grep only)
#   - scripts/ops/fast-check.sh    — quick developer smoke test (check + test + doctest)
#   - scripts/ops/install-hooks.sh — one-time setup to wire hooks into place
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
    if mdbook build docs/ >/dev/null 2>&1; then
        echo -e "${GREEN}ok${NC}"
    else
        echo -e "${RED}FAILED${NC}"
        echo -e "  ${YELLOW}Fix: mdbook build docs/${NC}"
        failed=1
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
