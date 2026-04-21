#!/usr/bin/env bash
#
# Recurring cleanup report for the rust-daq repository.
#
# Reports on:
#   - Oversized Rust files (>500 lines)
#   - Documentation drift (calls check-doc-drift.sh and check-inventory-drift.sh)
#   - TODO/FIXME count in source
#   - Deprecated item count (#[deprecated])
#   - Generated vs tracked file sizes
#   - Tracked raw debug artifacts
#
# This is a reporting tool — it always exits 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT" || exit 0

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

section() {
    echo ""
    echo "========================================"
    echo "  $1"
    echo "========================================"
}

tracked_rust_files() {
    git ls-files 'crates/**/*.rs' 2>/dev/null | sort
}

tracked_raw_debug_artifacts() {
    git ls-files \
        'debug/**/*.tiff' \
        'debug/**/*.hdf5' \
        'debug/**/*.fits' \
        'debug/**/.DS_Store' \
        'debug/.DS_Store' 2>/dev/null | sort
}

# ---------------------------------------------------------------------------
# 1. Oversized Rust files (>500 lines)
# ---------------------------------------------------------------------------

section "Oversized Rust Files (>500 lines)"

oversized_count=0
while IFS= read -r file; do
    lines=$(wc -l < "$file" | tr -d ' ')
    if [ "$lines" -gt 500 ]; then
        printf "  %6d lines  %s\n" "$lines" "$file"
        oversized_count=$((oversized_count + 1))
    fi
done < <(tracked_rust_files)

if [ "$oversized_count" -eq 0 ]; then
    echo "  None found."
else
    echo ""
    echo "  Total: $oversized_count file(s) over 500 lines"
fi

# ---------------------------------------------------------------------------
# 2. Documentation drift
# ---------------------------------------------------------------------------

section "Documentation Drift"

if [ -x scripts/hygiene/check-doc-drift.sh ]; then
    echo "--- check-doc-drift.sh ---"
    bash scripts/hygiene/check-doc-drift.sh 2>&1 | sed 's/^/  /'
    echo "  Exit code: $?"
else
    echo "  scripts/hygiene/check-doc-drift.sh not found or not executable — skipping."
fi

echo ""

if [ -x scripts/hygiene/check-inventory-drift.sh ]; then
    echo "--- check-inventory-drift.sh ---"
    bash scripts/hygiene/check-inventory-drift.sh 2>&1 | sed 's/^/  /'
    echo "  Exit code: $?"
else
    echo "  scripts/hygiene/check-inventory-drift.sh not found or not executable — skipping."
fi

echo ""

if [ -x scripts/hygiene/check-llm-wiki-drift.sh ]; then
    echo "--- check-llm-wiki-drift.sh ---"
    bash scripts/hygiene/check-llm-wiki-drift.sh 2>&1 | sed 's/^/  /'
    echo "  Exit code: $?"
else
    echo "  scripts/hygiene/check-llm-wiki-drift.sh not found or not executable — skipping."
fi

# ---------------------------------------------------------------------------
# 3. TODO / FIXME count
# ---------------------------------------------------------------------------

section "TODO / FIXME Count"

todo_count=0
fixme_count=0
hack_count=0

if command -v rg &>/dev/null; then
    todo_count=$(rg -c --type rust 'TODO' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
    fixme_count=$(rg -c --type rust 'FIXME' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
    hack_count=$(rg -c --type rust 'HACK|XXX' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
else
    todo_count=$(grep -r --include='*.rs' -c 'TODO' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
    fixme_count=$(grep -r --include='*.rs' -c 'FIXME' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
    hack_count=$(grep -r --include='*.rs' -c 'HACK\|XXX' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
fi

printf "  TODO:       %d\n" "$todo_count"
printf "  FIXME:      %d\n" "$fixme_count"
printf "  HACK/XXX:   %d\n" "$hack_count"
printf "  Combined:   %d\n" "$((todo_count + fixme_count + hack_count))"

# ---------------------------------------------------------------------------
# 4. Deprecated items
# ---------------------------------------------------------------------------

section "Deprecated Items"

if command -v rg &>/dev/null; then
    deprecated_count=$(rg -c --type rust '#\[deprecated' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
else
    deprecated_count=$(grep -r --include='*.rs' -c '#\[deprecated' crates/ 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
fi

printf "  #[deprecated] attributes: %d\n" "$deprecated_count"

if [ "$deprecated_count" -gt 0 ]; then
    echo ""
    echo "  Files with deprecated items:"
    if command -v rg &>/dev/null; then
        rg -l --type rust '#\[deprecated' crates/ 2>/dev/null | sed 's/^/    /'
    else
        grep -rl --include='*.rs' '#\[deprecated' crates/ 2>/dev/null | sed 's/^/    /'
    fi
fi

# ---------------------------------------------------------------------------
# 5. Generated vs tracked file sizes
# ---------------------------------------------------------------------------

section "Generated vs Tracked File Sizes"

# Proto-generated code (*.rs files in src/gen or similar generated paths)
proto_generated=0
proto_generated_bytes=0
while IFS= read -r f; do
    proto_generated=$((proto_generated + 1))
    size=$(wc -c < "$f" | tr -d ' ')
    proto_generated_bytes=$((proto_generated_bytes + size))
done < <(tracked_rust_files | grep -E '/src/gen[^/]*/' || true)

# Total tracked Rust source
total_rs_bytes=0
total_rs_count=0
while IFS= read -r f; do
    total_rs_count=$((total_rs_count + 1))
    size=$(wc -c < "$f" | tr -d ' ')
    total_rs_bytes=$((total_rs_bytes + size))
done < <(tracked_rust_files)

fmt_kb() {
    local bytes=$1
    if [ "$bytes" -ge 1048576 ]; then
        echo "$((bytes / 1048576)) MB"
    elif [ "$bytes" -ge 1024 ]; then
        echo "$((bytes / 1024)) KB"
    else
        echo "${bytes} B"
    fi
}

printf "  Total tracked .rs files:    %d (%s)\n" "$total_rs_count" "$(fmt_kb "$total_rs_bytes")"
printf "  Generated (src/gen*):       %d (%s)\n" "$proto_generated" "$(fmt_kb "$proto_generated_bytes")"

if [ "$total_rs_bytes" -gt 0 ] && [ "$proto_generated_bytes" -gt 0 ]; then
    pct=$((proto_generated_bytes * 100 / total_rs_bytes))
    printf "  Generated / total ratio:    %d%%\n" "$pct"
fi

# ---------------------------------------------------------------------------
# 6. Tracked raw debug artifacts
# ---------------------------------------------------------------------------

section "Tracked Raw Debug Artifacts"

raw_debug_count=0
raw_debug_bytes=0
while IFS= read -r f; do
    raw_debug_count=$((raw_debug_count + 1))
    if [ -f "$f" ]; then
        size=$(wc -c < "$f" | tr -d ' ')
        raw_debug_bytes=$((raw_debug_bytes + size))
        printf "  %8s  %s\n" "$(fmt_kb "$size")" "$f"
    else
        printf "  missing  %s\n" "$f"
    fi
done < <(tracked_raw_debug_artifacts)

if [ "$raw_debug_count" -eq 0 ]; then
    echo "  None found."
else
    echo ""
    printf "  Total: %d file(s), %s\n" "$raw_debug_count" "$(fmt_kb "$raw_debug_bytes")"
    echo "  Move durable fixtures under testdata/ or a named fixture path."
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

section "Summary"

echo "  Oversized files:    $oversized_count"
echo "  TODO+FIXME+HACK:    $((todo_count + fixme_count + hack_count))"
echo "  Deprecated items:   $deprecated_count"
echo "  Generated files:    $proto_generated"
echo "  Raw debug artifacts: $raw_debug_count"
echo ""
echo "  Report complete. Review items above for periodic cleanup."

# Always exit 0 — this is a reporting tool, not a gate.
exit 0
