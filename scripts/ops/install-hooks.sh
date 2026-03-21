#!/usr/bin/env bash
# Install git hooks for rust-daq
#
# Usage:
#   bash scripts/ops/install-hooks.sh [quick]
#
# Arguments:
#   quick - Install quick pre-commit hooks (formatting only, faster for development)
#
# Hook architecture:
#   - core.hooksPath = .beads/hooks (managed by beads, tracked in git)
#   - pre-commit: beads export → pre-commit framework (fmt/lint) → ast-grep
#   - pre-push:   beads sync check → quality gate (fmt + clippy + tests)
#   - post-merge/post-checkout/prepare-commit-msg: beads only
#
# The pre-push quality gate mirrors CI to prevent push failures.
# See scripts/ci/pre-push-gate.sh for the quality gate implementation.

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

echo "Installing git hooks for rust-daq..."

# ── 1. Verify beads hooks are active ────────────────────────────
echo ""
echo "=== Hook infrastructure ==="

HOOKS_PATH=$(git config --get core.hooksPath 2>/dev/null || echo "")
if [ "$HOOKS_PATH" = ".beads/hooks" ]; then
    echo -e "${GREEN}  core.hooksPath = .beads/hooks (beads-managed)${NC}"
else
    echo -e "${YELLOW}  core.hooksPath is not set to .beads/hooks${NC}"
    echo "  Run 'bd hooks install' first to set up beads hooks."
    echo "  Then re-run this script."
    exit 1
fi

# ── 2. Install pre-commit framework ─────────────────────────────
echo ""
echo "=== Pre-commit framework ==="

if ! command -v pre-commit &> /dev/null; then
    echo -e "${YELLOW}  pre-commit not found. Attempting to install...${NC}"
    if command -v pip3 &> /dev/null; then
        pip3 install pre-commit
    elif command -v pip &> /dev/null; then
        pip install pre-commit
    else
        echo -e "${RED}  Error: pip not found. Install pre-commit manually:${NC}"
        echo "    pip install pre-commit  OR  brew install pre-commit"
        exit 1
    fi
fi
echo -e "${GREEN}  pre-commit is installed${NC}"

# Install pre-commit hooks into .beads/hooks/ (respecting core.hooksPath)
CONFIG_FILE=".pre-commit-config.yaml"
if [ "$1" = "quick" ]; then
    CONFIG_FILE=".pre-commit-quick.yaml"
    echo "  Using quick config (format + ast-grep only)"
else
    echo "  Using full config (format, clippy, tests)"
fi

# pre-commit install writes to core.hooksPath when set
pre-commit install --config "$CONFIG_FILE" --hook-type pre-commit --allow-missing-config 2>/dev/null \
    || echo -e "${YELLOW}  pre-commit install skipped (core.hooksPath conflict)${NC}"
echo -e "${GREEN}  Pre-commit framework configured${NC}"

# ── 3. Verify pre-push gate ─────────────────────────────────────
echo ""
echo "=== Pre-push quality gate ==="

if [ -x "$REPO_ROOT/scripts/ci/pre-push-gate.sh" ]; then
    echo -e "${GREEN}  scripts/ci/pre-push-gate.sh is executable${NC}"
else
    echo -e "${RED}  scripts/ci/pre-push-gate.sh is missing or not executable${NC}"
    exit 1
fi

if grep -q "pre-push-gate.sh" "$REPO_ROOT/.beads/hooks/pre-push" 2>/dev/null; then
    echo -e "${GREEN}  .beads/hooks/pre-push chains quality gate${NC}"
else
    echo -e "${YELLOW}  Warning: .beads/hooks/pre-push does not reference pre-push-gate.sh${NC}"
    echo "  The composite pre-push hook should already be committed."
    echo "  Check: git show HEAD:.beads/hooks/pre-push"
fi

# ── Summary ──────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}All hooks verified.${NC}"
echo ""
echo "Hook behavior:"
echo "  git commit  -> beads export + pre-commit (format, ast-grep)"
echo "  git push    -> beads sync + quality gate (fmt, clippy, tests)"
echo ""
echo "Commands:"
echo "  bash scripts/ci/pre-push-gate.sh    # Run quality gate manually"
echo "  git push --no-verify             # Skip hooks (emergency only)"
