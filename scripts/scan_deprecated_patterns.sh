#!/bin/bash
# Deprecated Pattern Scanner for rust-daq PRs
# Identifies PRs using legacy actor model patterns that conflict with V3 migration
# Usage: ./scripts/scan_deprecated_patterns.sh

set -e

# Keywords indicating legacy actor model usage
# Based on Gemini's recommendation and Codex's analysis
DEPRECATED_PATTERNS=(
    "DaqManagerActor"
    "app_actor"
    "server_actor"
    "actix"
    "Actor"
    "Handler"
    "Context"
    "Addr"
    "InstrumentMeasurement"  # V1/V2 measurement type
    "V2InstrumentAdapter"    # Temporary adapter pattern
)

echo "==================================="
echo "Deprecated Pattern Scanner"
echo "==================================="
echo ""
echo "Scanning open PRs for legacy actor model patterns..."
echo "Patterns: ${DEPRECATED_PATTERNS[*]}"
echo ""

# Get list of open PR numbers
PR_NUMBERS=($(gh pr list --json number --jq '.[].number'))

if [[ ${#PR_NUMBERS[@]} -eq 0 ]]; then
    echo "No open PRs found."
    exit 0
fi

echo "Scanning ${#PR_NUMBERS[@]} PRs..."
echo ""

FOUND_COUNT=0

for pr in "${PR_NUMBERS[@]}"; do
    echo -n "PR #$pr..."

    # Get PR diff
    diff=$(gh pr diff "$pr" 2>/dev/null || echo "")

    if [[ -z "$diff" ]]; then
        echo " (no diff available)"
        continue
    fi

    found_patterns=()

    for pattern in "${DEPRECATED_PATTERNS[@]}"; do
        if echo "$diff" | grep -q "$pattern"; then
            found_patterns+=("$pattern")
        fi
    done

    if [[ ${#found_patterns[@]} -gt 0 ]]; then
        FOUND_COUNT=$((FOUND_COUNT + 1))

        # Get PR title for context
        pr_title=$(gh pr view "$pr" --json title --jq '.title')

        echo " ⚠️  FOUND: ${found_patterns[*]}"
        echo "   Title: $pr_title"
        echo "   Recommendation: Investigate for V1/actor model conflict"
        echo "   This PR may need to be closed or reimplemented for V3"
        echo ""
    else
        echo " ✓ Clean"
    fi
done

echo ""
echo "Summary:"
echo "--------"
echo "Total PRs scanned: ${#PR_NUMBERS[@]}"
echo "PRs with deprecated patterns: $FOUND_COUNT"

if [[ $FOUND_COUNT -gt 0 ]]; then
    echo ""
    echo "⚠️  Action Required:"
    echo "PRs with deprecated patterns should be reviewed against V3 migration goals."
    echo "See GEMINI_ARCHITECTURAL_ANALYSIS_2025-11-11.md for architectural guidance."
    exit 1
else
    echo ""
    echo "✓ All PRs appear to be V3-compatible!"
    exit 0
fi
