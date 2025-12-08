#!/bin/bash
# Cleanup Script for Tier 1 Trash Documentation
# Removes 30 AI session artifacts, old guides, and duplicate files
# Created: 2025-12-03

set -e  # Exit on error

REPO_ROOT="/Users/briansquires/code/rust-daq"
BACKUP_DIR="/tmp/rust-daq-docs-backup-$(date +%Y%m%d-%H%M%S)"

echo "=========================================="
echo "Trash Documentation Cleanup Script"
echo "=========================================="
echo ""
echo "This will DELETE 30 trash files:"
echo "  - 14 AI session artifacts"
echo "  - 10 old/outdated guides (Oct-Nov)"
echo "  - 3 auto-generated READMEs"
echo "  - 3 AI guide files"
echo ""
echo "Backup location: $BACKUP_DIR"
echo ""

# Define all Tier 1 trash files
declare -a TIER1_TRASH=(
    # AI Session Artifacts (14 files)
    "FINAL_CONSENSUS_REPORT.md"
    "PROJECT_STATE_REPORT.md"
    "SESSION_COMPLETE_SUMMARY.md"
    "HARDWARE_VALIDATION_SUMMARY.md"
    "KAMEO_INTEGRATION_PLAN.md"
    "IMPLEMENTATION_SUMMARY.md"
    "PHASE1_SUMMARY.md"
    "clients/python/IMPLEMENTATION_REPORT.md"
    "tests/JULES_TEST_PLAN.md"
    "tests/JULES_BRANCH_TEST_RESULTS.md"
    "clients/python/LAYER2_IMPLEMENTATION.md"
    "clients/python/NEW_README.md"
    "COCOINDEX_SETUP_COMPLETE.md"
    "DEVELOPMENT_TOOLS_SETUP_COMPLETE.md"

    # Old/Outdated Guides (10 files)
    "docs/guides/deployment/rust-daq-deployment.md"
    "docs/guides/deployment/README_DEPLOYMENT.md"
    "docs/guides/testing/HARDWARE_IN_THE_LOOP_TESTING.md"
    "docs/guides/testing/tests_README.md"
    "docs/guides/ci_cd/TAILSCALE_SETUP.md"
    "docs/guides/measurement-processor-guide.md"
    "docs/guides/rust-daq-data-guide.md"
    "docs/guides/rust-daq-instrument-guide.md"
    "docs/guides/rust-daq-performance-test.md"
    "docs/guides/hdf5_storage_guide.md"

    # Auto-generated/Tool READMEs (3 files)
    "clients/python/.pytest_cache/README.md"
    "clients/python/notebooks/README.md"
    "examples/scripts/README.md"

    # AI Guide Files (3 files)
    "AGENTS.md"
    "AI_AGENT_DEVELOPMENT_GUIDE.md"
    "GEMINI.md"
)

echo "Files to be deleted:"
echo "===================="
for file in "${TIER1_TRASH[@]}"; do
    if [ -f "$REPO_ROOT/$file" ]; then
        echo "  ✓ $file"
    else
        echo "  ✗ $file (NOT FOUND)"
    fi
done
echo ""

# Count existing files
existing_count=0
for file in "${TIER1_TRASH[@]}"; do
    if [ -f "$REPO_ROOT/$file" ]; then
        ((existing_count++))
    fi
done

echo "Summary:"
echo "  Files to delete: ${#TIER1_TRASH[@]}"
echo "  Files found: $existing_count"
echo "  Files missing: $((${#TIER1_TRASH[@]} - existing_count))"
echo ""

# Ask for confirmation
read -p "Proceed with deletion? (yes/no): " confirmation
if [ "$confirmation" != "yes" ]; then
    echo "Aborted."
    exit 0
fi

echo ""
echo "Creating backup..."
mkdir -p "$BACKUP_DIR"

# Backup and delete files
deleted_count=0
for file in "${TIER1_TRASH[@]}"; do
    full_path="$REPO_ROOT/$file"

    if [ -f "$full_path" ]; then
        # Create backup directory structure
        backup_path="$BACKUP_DIR/$(dirname "$file")"
        mkdir -p "$backup_path"

        # Backup file
        cp "$full_path" "$BACKUP_DIR/$file"

        # Delete file
        rm "$full_path"
        echo "  Deleted: $file"
        ((deleted_count++))
    fi
done

echo ""
echo "=========================================="
echo "Cleanup Complete!"
echo "=========================================="
echo "  Files deleted: $deleted_count"
echo "  Backup location: $BACKUP_DIR"
echo ""
echo "Next steps:"
echo "  1. Review remaining docs: find docs -name '*.md' -type f"
echo "  2. Update CocoIndex flow to exclude more trash (Tier 2)"
echo "  3. Run: git status"
echo ""
echo "To restore backup (if needed):"
echo "  cp -r $BACKUP_DIR/* $REPO_ROOT/"
echo ""
