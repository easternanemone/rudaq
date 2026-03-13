#!/usr/bin/env bash
#
# Check for drift between workspace metadata and reference documentation.
# Compares cargo metadata (crate list + feature definitions) against:
#   - docs/reference/inventory.md   (Crate Layout table)
#   - docs/reference/feature-matrix.md (feature entries)
#
# Exits 0 if no drift, 1 if drift detected.

set -euo pipefail

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------

if ! command -v jq &>/dev/null; then
    echo "Error: jq is required but not found. Install it via your package manager."
    exit 1
fi

if [ ! -f "Cargo.toml" ] || [ ! -d "docs/reference" ]; then
    echo "Error: Run this script from the repository root."
    exit 1
fi

INVENTORY="docs/reference/inventory.md"
FEATURE_MATRIX="docs/reference/feature-matrix.md"

if [ ! -f "$INVENTORY" ]; then
    echo "Error: $INVENTORY not found."
    exit 1
fi

if [ ! -f "$FEATURE_MATRIX" ]; then
    echo "Error: $FEATURE_MATRIX not found."
    exit 1
fi

DRIFT=0

# Cache cargo metadata (single invocation for the whole script)
CARGO_META=$(cargo metadata --no-deps --format-version=1 2>/dev/null)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

section() {
    echo ""
    echo "=== $1 ==="
}

pass() {
    echo "  PASS: $1"
}

fail() {
    echo "  DRIFT: $1"
    DRIFT=1
}

# Extract data rows from a markdown table (skip header and separator rows).
# Reads from stdin, writes to stdout.
table_data_rows() {
    grep '^|' \
        | grep -v '^| *---' \
        | grep -v '^| *Feature' \
        | grep -v '^| *Profile' \
        | grep -v '^| *Flag' \
        | grep -v '^| *Name' \
        || true
}

# Extract the first backtick-wrapped token from each line.
# Reads from stdin, writes one name per line to stdout.
first_backtick_token() {
    sed 's/^| *//' | grep -oE '^`[a-z][a-z0-9_-]*`' | tr -d '`' || true
}

# ---------------------------------------------------------------------------
# 1. Crate inventory check
# ---------------------------------------------------------------------------

section "Crate Inventory (docs/reference/inventory.md)"

# Get workspace crate names from cargo metadata
METADATA_CRATES=$(echo "$CARGO_META" | jq -r '.packages[].name' | sort)

# Strategy for extracting documented crates from inventory.md:
#
# 1. Extract backtick-wrapped names and glob patterns from the Crate Layout table.
#    These include explicit names like `common`, `server` and glob patterns like
#    `driver-*`, `*-sys`.
#
# 2. Extract crate names from Binaries table paths (`crates/<name>` and `-p <name>`).
#
# 3. Combine all names and patterns, then match workspace crates against them.

# --- Crate Layout table ---
LAYOUT_SECTION=$(awk '/^## Crate Layout/{found=1; next} /^## /{if(found) exit} found{print}' "$INVENTORY")

# Extract backtick-wrapped tokens: allow leading * for glob patterns like `*-sys`
LAYOUT_CRATES=$(echo "$LAYOUT_SECTION" \
    | grep -oE '`[a-z*][a-z0-9_*-]*`' \
    | tr -d '`' \
    | sort -u)

# --- Binaries table: extract crate names from `crates/<name>` paths ---
BINARIES_SECTION=$(awk '/^## Binaries/{found=1; next} /^## /{if(found) exit} found{print}' "$INVENTORY")

BINARY_CRATES=$(echo "$BINARIES_SECTION" \
    | grep -oE 'crates/[a-z][a-z0-9_-]*' \
    | sed 's|crates/||' \
    | sort -u)

# Also extract from `-p <name>` cargo commands
BINARY_CRATES_P=$(echo "$BINARIES_SECTION" \
    | grep -oE '\-p [a-z][a-z0-9_-]*' \
    | sed 's/-p //' \
    | sort -u)

# --- Feature Flags table: extract crate owner names ---
FLAGS_SECTION=$(awk '/^## Feature Flags/{found=1; next} /^## /{if(found) exit} found{print}' "$INVENTORY")

FLAGS_CRATES=$(echo "$FLAGS_SECTION" \
    | grep -oE '`[a-z][a-z0-9_-]*`' \
    | tr -d '`' \
    | sort -u)

# Combine all names and patterns
ALL_DOC_TOKENS=$(printf '%s\n%s\n%s\n%s\n' \
    "$LAYOUT_CRATES" "$BINARY_CRATES" "$BINARY_CRATES_P" "$FLAGS_CRATES" \
    | sort -u | grep -v '^$')

# Separate glob patterns from explicit names
GLOB_PATTERNS=$(echo "$ALL_DOC_TOKENS" | grep '\*' || true)
EXPLICIT_NAMES=$(echo "$ALL_DOC_TOKENS" | grep -v '\*' || true)

# Match each workspace crate against documented names or glob patterns
UNMATCHED_METADATA=""

for crate in $METADATA_CRATES; do
    matched=false

    # Check explicit names
    if echo "$EXPLICIT_NAMES" | grep -qx "$crate"; then
        matched=true
    fi

    # Check glob patterns
    if ! $matched; then
        for pattern in $GLOB_PATTERNS; do
            # shellcheck disable=SC2254
            case "$crate" in
                $pattern) matched=true; break ;;
            esac
        done
    fi

    if ! $matched; then
        UNMATCHED_METADATA="$UNMATCHED_METADATA $crate"
    fi
done

UNMATCHED_METADATA=$(echo "$UNMATCHED_METADATA" | xargs)

if [ -z "$UNMATCHED_METADATA" ]; then
    pass "All $(echo "$METADATA_CRATES" | wc -l | xargs) workspace crates are covered in inventory.md"
else
    fail "Crates in workspace but NOT in inventory.md:"
    for c in $UNMATCHED_METADATA; do
        echo "    + $c"
    done
fi

# Check for explicit crate names in inventory that no longer exist in the workspace
# (skip glob patterns since they match dynamically)
STALE_INVENTORY=""
for name in $EXPLICIT_NAMES; do
    # Only check names from the Crate Layout or Binaries sections (avoid false
    # positives from feature-flag names or runtime modes).
    if echo "$LAYOUT_CRATES $BINARY_CRATES $BINARY_CRATES_P" | tr ' ' '\n' | grep -qx "$name"; then
        if ! echo "$METADATA_CRATES" | grep -qx "$name"; then
            STALE_INVENTORY="$STALE_INVENTORY $name"
        fi
    fi
done

STALE_INVENTORY=$(echo "$STALE_INVENTORY" | xargs)

if [ -z "$STALE_INVENTORY" ]; then
    pass "No stale crate names in inventory.md"
else
    fail "Crate names in inventory.md but NOT in workspace:"
    for c in $STALE_INVENTORY; do
        echo "    - $c"
    done
fi

# ---------------------------------------------------------------------------
# 2. Feature drift: driver-registry
# ---------------------------------------------------------------------------

section "Feature Drift: driver-registry (docs/reference/feature-matrix.md)"

# Get actual features from cargo metadata
DR_FEATURES_ACTUAL=$(echo "$CARGO_META" \
    | jq -r '.packages[] | select(.name == "driver-registry") | .features | keys[]' \
    | sort)

# Extract feature names from the Driver Registry Features table.
# Use awk to stop before the "Always included" text or Camera Hardware section.
DR_TABLE_FEATURES=$(awk '/^#### Driver Registry Features/{found=1; next} /^\*\*Always included|^### Camera Hardware|^---$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | table_data_rows \
    | first_backtick_token \
    | sort -u)

# Also extract features from the Feature Dependencies block for driver-registry
DR_DEPS_FEATURES=$(awk '/^driver-registry features:/{found=1; next} /^$|^```/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | sed 's/^[[:space:]]*//' \
    | cut -d' ' -f1 \
    | sort -u)

DR_FEATURES_DOC=$(printf '%s\n%s\n' "$DR_TABLE_FEATURES" "$DR_DEPS_FEATURES" \
    | sort -u | grep -v '^$')

# Compare
DR_MISSING_DOC=""
for feat in $DR_FEATURES_ACTUAL; do
    if [ "$feat" = "default" ]; then
        continue  # 'default' is implicit, not typically documented as a row
    fi
    if ! echo "$DR_FEATURES_DOC" | grep -qx "$feat"; then
        DR_MISSING_DOC="$DR_MISSING_DOC $feat"
    fi
done
DR_MISSING_DOC=$(echo "$DR_MISSING_DOC" | xargs)

DR_STALE_DOC=""
for feat in $DR_FEATURES_DOC; do
    if ! echo "$DR_FEATURES_ACTUAL" | grep -qx "$feat"; then
        DR_STALE_DOC="$DR_STALE_DOC $feat"
    fi
done
DR_STALE_DOC=$(echo "$DR_STALE_DOC" | xargs)

if [ -z "$DR_MISSING_DOC" ]; then
    pass "All driver-registry features documented"
else
    fail "driver-registry features NOT in feature-matrix.md:"
    for f in $DR_MISSING_DOC; do
        echo "    + $f"
    done
fi

if [ -z "$DR_STALE_DOC" ]; then
    pass "No stale driver-registry features in docs"
else
    fail "Documented driver-registry features NOT in Cargo.toml:"
    for f in $DR_STALE_DOC; do
        echo "    - $f"
    done
fi

# ---------------------------------------------------------------------------
# 3. Feature drift: bin crate
# ---------------------------------------------------------------------------

section "Feature Drift: bin (docs/reference/feature-matrix.md)"

BIN_FEATURES_ACTUAL=$(echo "$CARGO_META" \
    | jq -r '.packages[] | select(.name == "bin") | .features | keys[]' \
    | sort)

# Extract bin features from multiple sections of feature-matrix.md:
#   - High-Level Profiles table (profile names = feature names)
#   - Device Drivers (bin crate) table (stops at sub-sections)
#   - System Features table
#   - Default Features code block
#   - Feature Dependencies (bin crate features) section

# Profiles table: first column = feature name
PROFILES_FEATURES=$(awk '/^## High-Level Profiles/{found=1; next} /^---$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | table_data_rows \
    | first_backtick_token)

# Device Drivers (bin crate) table — stop at the #### sub-heading
BIN_DRIVERS_FEATURES=$(awk '/^### Device Drivers \(bin crate\)/{found=1; next} /^####|^###[^#]|^---$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | table_data_rows \
    | first_backtick_token)

# System Features table
SYSTEM_FEATURES=$(awk '/^## System Features/{found=1; next} /^---$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | table_data_rows \
    | first_backtick_token)

# Default Features code block
DEFAULT_FEATURES=$(awk '/^## Default Features/{found=1; next} /^---$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | grep -oE '"[a-z][a-z0-9_-]*"' \
    | tr -d '"' \
    || true)

# Feature Dependencies — left-hand side of arrows in "bin crate features:" block
DEPS_FEATURES=$(awk '/^bin crate features:/{found=1; next} /^$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | sed 's/^[[:space:]]*//' \
    | cut -d' ' -f1)

# Combine all documented bin features
BIN_FEATURES_DOC=$(printf '%s\n%s\n%s\n%s\n%s\n' \
    "$PROFILES_FEATURES" "$BIN_DRIVERS_FEATURES" "$SYSTEM_FEATURES" \
    "$DEFAULT_FEATURES" "$DEPS_FEATURES" \
    | sort -u | grep -v '^$')

# Compare — only check features that are on the bin crate itself
BIN_MISSING_DOC=""
for feat in $BIN_FEATURES_ACTUAL; do
    if [ "$feat" = "default" ]; then
        continue
    fi
    if ! echo "$BIN_FEATURES_DOC" | grep -qx "$feat"; then
        BIN_MISSING_DOC="$BIN_MISSING_DOC $feat"
    fi
done
BIN_MISSING_DOC=$(echo "$BIN_MISSING_DOC" | xargs)

BIN_STALE_DOC=""
for feat in $BIN_FEATURES_DOC; do
    if ! echo "$BIN_FEATURES_ACTUAL" | grep -qx "$feat"; then
        BIN_STALE_DOC="$BIN_STALE_DOC $feat"
    fi
done
BIN_STALE_DOC=$(echo "$BIN_STALE_DOC" | xargs)

if [ -z "$BIN_MISSING_DOC" ]; then
    pass "All bin features documented"
else
    fail "bin features NOT in feature-matrix.md:"
    for f in $BIN_MISSING_DOC; do
        echo "    + $f"
    done
fi

if [ -z "$BIN_STALE_DOC" ]; then
    pass "No stale bin features in docs"
else
    fail "Documented bin features NOT in Cargo.toml:"
    for f in $BIN_STALE_DOC; do
        echo "    - $f"
    done
fi

# ---------------------------------------------------------------------------
# 4. Feature drift: storage crate
# ---------------------------------------------------------------------------

section "Feature Drift: storage (docs/reference/feature-matrix.md)"

STORAGE_FEATURES_ACTUAL=$(echo "$CARGO_META" \
    | jq -r '.packages[] | select(.name == "storage") | .features | keys[]' \
    | sort)

# Extract storage features from Storage Backends section
STORAGE_FEATURES_DOC=$(awk '/^## Storage Backends/{found=1; next} /^---$/{if(found) exit} found{print}' "$FEATURE_MATRIX" \
    | table_data_rows \
    | first_backtick_token \
    | sort -u)

STORAGE_MISSING_DOC=""
for feat in $STORAGE_FEATURES_ACTUAL; do
    if [ "$feat" = "default" ]; then
        continue
    fi
    if ! echo "$STORAGE_FEATURES_DOC" | grep -qx "$feat"; then
        STORAGE_MISSING_DOC="$STORAGE_MISSING_DOC $feat"
    fi
done
STORAGE_MISSING_DOC=$(echo "$STORAGE_MISSING_DOC" | xargs)

STORAGE_STALE_DOC=""
for feat in $STORAGE_FEATURES_DOC; do
    if ! echo "$STORAGE_FEATURES_ACTUAL" | grep -qx "$feat"; then
        STORAGE_STALE_DOC="$STORAGE_STALE_DOC $feat"
    fi
done
STORAGE_STALE_DOC=$(echo "$STORAGE_STALE_DOC" | xargs)

if [ -z "$STORAGE_MISSING_DOC" ]; then
    pass "All storage features documented"
else
    fail "storage features NOT in feature-matrix.md:"
    for f in $STORAGE_MISSING_DOC; do
        echo "    + $f"
    done
fi

if [ -z "$STORAGE_STALE_DOC" ]; then
    pass "No stale storage features in docs"
else
    fail "Documented storage features NOT in Cargo.toml:"
    for f in $STORAGE_STALE_DOC; do
        echo "    - $f"
    done
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

section "Summary"

if [ "$DRIFT" -ne 0 ]; then
    echo "  Inventory drift detected. Please update the documentation."
    exit 1
else
    echo "  All inventory checks passed. Documentation is in sync."
    exit 0
fi
