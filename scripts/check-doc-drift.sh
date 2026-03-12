#!/usr/bin/env bash
#
# Check for common documentation drift patterns.
# Run this from the root of the repository.
# Exits with 1 if any stale patterns are found.

set -euo pipefail

DOCS_DIR="docs"
ROOT_MD_FILES="README.md CLAUDE.md GEMINI.md ANDOR_SDK_FIXES.md"

if [ ! -d "$DOCS_DIR" ]; then
    echo "Error: Run this script from the repository root."
    exit 1
fi

STALE_PATTERNS=(
    "rust-daq-server:Replaced by rust-daq-daemon (crates/bin)"
    "\bmock_all\.toml\b:Replaced by demo_mock_all.toml"
    "comedi_hardware:Replaced by comedi-sdk or hardware"
    "maitai_test\.toml:Replaced by maitai_declarative_test.toml"
)

ERRORS=0

echo "Running documentation drift checks..."

# We search inside docs/ and the root markdown files
FILES_TO_CHECK=$(find "$DOCS_DIR" -type f -name "*.md")
FILES_TO_CHECK="$FILES_TO_CHECK $ROOT_MD_FILES"

for file in $FILES_TO_CHECK; do
    if [ ! -f "$file" ]; then
        continue
    fi

    # Skip archival docs and references to old patterns in this script itself or history
    if grep -q "\[!WARNING\] \*\*ARCHIVAL / HISTORICAL\*\*" "$file" || [[ "$file" == *"HISTORY.md"* ]]; then
        continue
    fi

    for entry in "${STALE_PATTERNS[@]}"; do
        PATTERN="${entry%%:*}"
        MSG="${entry##*:}"

        # We use ripgrep or standard grep
        if grep -HnE "$PATTERN" "$file" > /dev/null 2>&1; then
            echo "::error file=$file::Stale pattern found: '$PATTERN'. $MSG"
            grep -Hn --color=always -E "$PATTERN" "$file" | sed 's/^/  /'
            ERRORS=1
        fi
    done
done

if [ "$ERRORS" -ne 0 ]; then
    echo "Doc drift checks failed. Please update the stale references above."
    exit 1
else
    echo "Doc drift checks passed."
fi
