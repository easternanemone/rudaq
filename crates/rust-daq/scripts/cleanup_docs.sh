#!/bin/bash

# ==============================================================================
# RUST-DAQ DOCUMENTATION CLEANUP SCRIPT
# ==============================================================================
#
# ANALYSIS OF "GOLD STANDARD":
# The "Gold Standard" documentation set should reflect the *active* V5 Headless
# architecture.
#
# CRITICAL RESCUE OPS:
# 1. `docs/v4/CONFIG_SYSTEM.md`: Your `src/lib.rs` still exports `config_v4`.
#    This is the CURRENT config schema documentation. It must be kept and
#    renamed to `docs/architecture/configuration.md`.
# 2. `docs/v4/TRACING_SYSTEM.md`: Similarly, `tracing_v4` is active.
#    Renaming to `docs/architecture/tracing.md`.
# 3. `docs/v4/TRAIT_USAGE_GUIDE.md`: Active trait usage patterns.
#    Renamed to `docs/guides/trait_usage.md`.
# 4. `docs/v4/PHASE_1D_TRAIT_UPDATES.md`: Post-hardware validation learnings (Nov 2025).
#    Contains current SerialAdapterV4Builder patterns for V5.
#    Renamed to `docs/architecture/v5_trait_patterns.md`.
# 5. `docs/HEADLESS_FIRST_ARCHITECTURE.md`: ACTIVE V5 architecture master doc.
#    Renamed to `docs/architecture/v5_headless.md`.
# 6. `docs/CONFIGURATION_ARCHITECTURE.md`: V5 Figment/Serde config system.
#    Renamed to `docs/architecture/v5_configuration.md`.
# 7. `docs/V5_HARDWARE_INTEGRATION_STATUS.md`: Current hardware integration work.
#    Renamed to `docs/project_management/V5_HARDWARE_STATUS.md`.
#
# ==============================================================================

# 1. Setup Directory Structure
mkdir -p docs/archive
mkdir -p docs/architecture/v5_reference

# ------------------------------------------------------------------------------
# SECTION A: DELETE (Garbage Collection)
# ------------------------------------------------------------------------------
# These are temporary outputs, git logs, or duplicate text files.
# ------------------------------------------------------------------------------
rm -f docs/branch-cherry-pick-analysis.md
rm -f docs/branch-cleanup-summary.md
rm -f docs/cherry-pick-final-report.md
rm -f docs/final-branch-investigation.md
rm -f docs/DISCOVERY_TOOL_RESULTS_*.md
rm -f docs/BD_51B1_DESIGN_COMPLETE.txt
rm -f docs/project_management/CRITICAL_FIXES_APPLIED.md # One-off log
rm -f docs/project_management/DEPLOYMENT_QUEUE.md       # Stale queue

# ------------------------------------------------------------------------------
# SECTION B: RESCUE & REFACTOR (Move to Active Docs)
# ------------------------------------------------------------------------------
# These files were marked for archive but describe ACTIVE code in src/lib.rs.
# We rename them to generic names to drop the "v4" stigma.
# ------------------------------------------------------------------------------

# Config System (Active in src/config_v4.rs)
if [ -f "docs/v4/CONFIG_SYSTEM.md" ]; then
  mv docs/v4/CONFIG_SYSTEM.md docs/architecture/configuration.md
  echo "Rescued CONFIG_SYSTEM.md -> docs/architecture/configuration.md"
fi

# Tracing System (Active in src/tracing_v4.rs)
if [ -f "docs/v4/TRACING_SYSTEM.md" ]; then
  mv docs/v4/TRACING_SYSTEM.md docs/architecture/tracing.md
  echo "Rescued TRACING_SYSTEM.md -> docs/architecture/tracing.md"
fi

# Traits Guide (Active usage in src/traits)
if [ -f "docs/v4/TRAIT_USAGE_GUIDE.md" ]; then
  mv docs/v4/TRAIT_USAGE_GUIDE.md docs/guides/trait_usage.md
  echo "Rescued TRAIT_USAGE_GUIDE.md -> docs/guides/trait_usage.md"
fi

# V5 Trait Patterns (Post-hardware validation learnings - Nov 2025)
if [ -f "docs/v4/PHASE_1D_TRAIT_UPDATES.md" ]; then
  mv docs/v4/PHASE_1D_TRAIT_UPDATES.md docs/architecture/v5_trait_patterns.md
  echo "Rescued PHASE_1D_TRAIT_UPDATES.md -> docs/architecture/v5_trait_patterns.md"
fi

# V5 Headless Architecture (ACTIVE - master architecture doc)
if [ -f "docs/HEADLESS_FIRST_ARCHITECTURE.md" ]; then
  mv docs/HEADLESS_FIRST_ARCHITECTURE.md docs/architecture/v5_headless.md
  echo "Rescued HEADLESS_FIRST_ARCHITECTURE.md -> docs/architecture/v5_headless.md"
fi

# V5 Configuration System (Figment/Serde integration)
if [ -f "docs/CONFIGURATION_ARCHITECTURE.md" ]; then
  mv docs/CONFIGURATION_ARCHITECTURE.md docs/architecture/v5_configuration.md
  echo "Rescued CONFIGURATION_ARCHITECTURE.md -> docs/architecture/v5_configuration.md"
fi

# V5 Status (This is current work, keep in PM)
if [ -f "docs/V5_HARDWARE_INTEGRATION_STATUS.md" ]; then
  mv docs/V5_HARDWARE_INTEGRATION_STATUS.md docs/project_management/V5_HARDWARE_STATUS.md
  echo "Preserved V5_HARDWARE_INTEGRATION_STATUS.md in project_management"
fi

# ------------------------------------------------------------------------------
# SECTION C: ARCHIVE (Historical Context)
# ------------------------------------------------------------------------------
# Reports, summaries, and "Plan" documents that are now completed/obsolete.
# ------------------------------------------------------------------------------

# Root Level clutter
mv FINAL_CONSENSUS_REPORT.md docs/archive/ 2>/dev/null
mv HARDWARE_VALIDATION_SUMMARY.md docs/archive/ 2>/dev/null
mv KAMEO_INTEGRATION_PLAN.md docs/archive/ 2>/dev/null
mv PROJECT_STATE_REPORT.md docs/archive/ 2>/dev/null
mv SESSION_COMPLETE_SUMMARY.md docs/archive/ 2>/dev/null
mv V4_GUI_INDEX.md docs/archive/ 2>/dev/null
mv V4_GUI_QUICK_REFERENCE.md docs/archive/ 2>/dev/null
mv V4_DAQ_WORKSPACE_SUMMARY.txt docs/archive/ 2>/dev/null
mv V4_WORKSPACE_FILE_LISTING.txt docs/archive/ 2>/dev/null

# Docs Folder: Old Phase Reports
# We use wildcards to catch the bulk of the status reports.
mv docs/ARCHITECTURAL_ANALYSIS_*.md docs/archive/ 2>/dev/null
mv docs/ARCHITECTURAL_REDESIGN_*.md docs/archive/ 2>/dev/null
mv docs/ARCHITECTURE_STATUS_*.md docs/archive/ 2>/dev/null
mv docs/AST_GREP_*.md docs/archive/ 2>/dev/null
mv docs/BD-*.md docs/archive/ 2>/dev/null
mv docs/BD_*.md docs/archive/ 2>/dev/null
mv docs/BLOCKING_LAYER_ANALYSIS.md docs/archive/ 2>/dev/null
mv docs/CAPABILITY_MIGRATION_GUIDE.md docs/archive/ 2>/dev/null
mv docs/CONSENSUS_REVIEW_*.md docs/archive/ 2>/dev/null
mv docs/E2E_VALIDATION_SUCCESS.md docs/archive/ 2>/dev/null
mv docs/ELL14_INTEGRATION_STATUS.md docs/archive/ 2>/dev/null # Specific status, superseded by docs/instruments/elliptec.md
mv docs/GUI_V2_IMPLEMENTATION_STATUS.md docs/archive/ 2>/dev/null
mv docs/HARDWARE_*.md docs/archive/ 2>/dev/null
mv docs/HEADLESS_MIGRATION_COMPLETE.md docs/archive/ 2>/dev/null
mv docs/HELPER_MODULES_V2_PLAN.md docs/archive/ 2>/dev/null
mv docs/PHASE*.md docs/archive/ 2>/dev/null
mv docs/PVCAM_V3_*.md docs/archive/ 2>/dev/null
mv docs/TASK_*.md docs/archive/ 2>/dev/null
mv docs/V2_*.md docs/archive/ 2>/dev/null
mv docs/V3_*.md docs/archive/ 2>/dev/null
mv docs/VISA_V2_IMPLEMENTATION.md docs/archive/ 2>/dev/null
mv docs/architectural-analysis-*.md docs/archive/ 2>/dev/null
mv docs/daq-28-phase2-spec.md docs/archive/ 2>/dev/null
mv docs/jules-phase2-postmortem.md docs/archive/ 2>/dev/null
mv docs/killer_features.md docs/archive/ 2>/dev/null
mv docs/pillar*_tasks.md docs/archive/ 2>/dev/null
mv docs/pixelbuffer-implementation-summary.md docs/archive/ 2>/dev/null
mv docs/task-*.md docs/archive/ 2>/dev/null

# Docs Folder: Remaining V4 GUI/Obsolete artifacts
# Note: We rescued the useful V4 config/tracing docs above.
# Anything left in docs/v4 is likely GUI integration or verification of the old actor system.
mv docs/v4_*.md docs/archive/ 2>/dev/null
if [ -d "docs/v4" ]; then
  mv docs/v4 docs/archive/v4_legacy
fi

# Project Management Logs
mv docs/project_management/PHASE_1B_HARDWARE_TESTING.md docs/archive/ 2>/dev/null
mv docs/project_management/ZEN_TOOLS_V4_EVALUATION.md docs/archive/ 2>/dev/null

echo "Cleanup complete."
echo " - Active architecture docs consolidated in docs/architecture/"
echo " - Active guides consolidated in docs/guides/"
echo " - Old reports moved to docs/archive/"
