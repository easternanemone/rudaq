#!/bin/bash
# Cleanup Script for Tier 2 Likely Trash Documentation
# Removes ~17 AI analyses, old research, redirects, and deprecated docs
# KEEPS: rust-daq-gui-guide.md (main egui docs)
# Created: 2025-12-03

set -e  # Exit on error

REPO_ROOT="/Users/briansquires/code/rust-daq"
BACKUP_DIR="/tmp/rust-daq-docs-tier2-backup-$(date +%Y%m%d-%H%M%S)"

echo "=========================================="
echo "Tier 2 Trash Documentation Cleanup"
echo "=========================================="
echo ""
echo "This will DELETE ~17 likely trash files:"
echo "  - Old V4 architecture research (3 files)"
echo "  - AI analysis snapshots (5 files)"
echo "  - Today's AI session artifacts (9 files)"
echo ""
echo "KEEPS: docs/guides/rust-daq-gui-guide.md (egui main docs)"
echo ""
echo "Backup location: $BACKUP_DIR"
echo ""

# Define all Tier 2 trash files
declare -a TIER2_TRASH=(
    # Old V4 Architecture Research/Analysis (3 files)
    "docs/architecture/ADDITIONAL_LIBRARY_RESEARCH.md"
    "docs/architecture/RUST_LIBRARY_RECOMMENDATIONS.md"
    "docs/architecture/hdf5_actor_design.md"

    # November Analysis Docs - AI snapshots/assessments (5 files)
    "docs/architecture/CODE_ANALYSIS_2025-11-25.md"
    "docs/architecture/GUI_DESIGN.md"
    "docs/architecture/ARCHITECTURE_COORDINATOR_INITIAL_ASSESSMENT.md"
    "docs/architecture/plugin_system_research.md"
    "docs/architecture/V5_OPTIMIZATION_STRATEGIES.md"

    # December AI Session Artifacts (9 files)
    "docs/COCOINDEX_INTEGRATION.md"           # AI integration summary
    "docs/CODEBASE_ANALYSIS.md"               # AI codebase analysis
    "docs/guides/client_examples.md"          # Likely AI-generated
    "docs/guides/GUI.md"                      # Redirect to rust-daq-gui-guide.md
    "docs/guides/egui_gui_quickstart.md"      # Redirect to rust-daq-gui-guide.md
    "docs/MORPH_AUTH_ISSUE.md"                # Session artifact about auth debugging
    "docs/architecture/V5_TRANSITION_COMPLETE.md"  # AI summary
    "COCOINDEX_QUICKSTART.md"                 # Duplicate of HYBRID_SEARCH_SETUP.md

    # Tauri GUI docs (deprecated/legacy) (3 files)
    "gui-tauri/QUICK_START.md"
    "gui-tauri/README.md"
    "gui-tauri/SEQUENCER_README.md"
)

echo "Files to be deleted:"
echo "===================="
for file in "${TIER2_TRASH[@]}"; do
    if [ -f "$REPO_ROOT/$file" ]; then
        echo "  ✓ $file"
    else
        echo "  ✗ $file (NOT FOUND)"
    fi
done
echo ""

# Count existing files
existing_count=0
for file in "${TIER2_TRASH[@]}"; do
    if [ -f "$REPO_ROOT/$file" ]; then
        ((existing_count++))
    fi
done

echo "Summary:"
echo "  Files to delete: ${#TIER2_TRASH[@]}"
echo "  Files found: $existing_count"
echo "  Files missing: $((${#TIER2_TRASH[@]} - existing_count))"
echo ""
echo "KEEPING (egui docs):"
echo "  ✓ docs/guides/rust-daq-gui-guide.md (main egui guide)"
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
for file in "${TIER2_TRASH[@]}"; do
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
echo "Tier 2 Cleanup Complete!"
echo "=========================================="
echo "  Files deleted: $deleted_count"
echo "  Backup location: $BACKUP_DIR"
echo ""
echo "Remaining markdown count: $(find $REPO_ROOT -name '*.md' -type f | grep -v target | grep -v .git | grep -v node_modules | grep -v .claude.DANGEROUS.backup | grep -v .venv | wc -l | tr -d ' ')"
echo ""
echo "Next steps:"
echo "  1. Review final docs: find docs -name '*.md' -type f | sort"
echo "  2. Run CocoIndex indexing with cleaned docs"
echo "  3. Run: git status"
echo ""
echo "To restore backup (if needed):"
echo "  cp -r $BACKUP_DIR/* $REPO_ROOT/"
echo ""
