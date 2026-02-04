#!/usr/bin/env bash
# Verification script for driver-dover-motion implementation
# Run this after committing to verify everything works

set -e  # Exit on error

echo "=== Dover Motion Driver Verification ==="
echo ""

# Navigate to repo root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$REPO_ROOT"

echo "Repository: $REPO_ROOT"
echo ""

# 1. Build driver in mock mode (default)
echo "1. Building driver-dover-motion (mock mode)..."
cargo build -p driver-dover-motion
echo "✓ Mock mode build successful"
echo ""

# 2. Build driver with hardware feature
echo "2. Building driver-dover-motion (hardware mode)..."
cargo build -p driver-dover-motion --features dover-hardware
echo "✓ Hardware mode build successful"
echo ""

# 3. Build common crate (verify TriggerOnPosition trait compiles)
echo "3. Building common crate..."
cargo build -p common
echo "✓ Common crate build successful"
echo ""

# 4. Run driver tests
echo "4. Running driver-dover-motion tests..."
if command -v cargo-nextest &> /dev/null; then
    cargo nextest run -p driver-dover-motion
else
    cargo test -p driver-dover-motion
fi
echo "✓ All driver tests passed"
echo ""

# 5. Run common crate tests (capabilities module)
echo "5. Running common crate capabilities tests..."
if command -v cargo-nextest &> /dev/null; then
    cargo nextest run -p common --lib capabilities
else
    cargo test -p common --lib capabilities
fi
echo "✓ Common capabilities tests passed"
echo ""

# 6. Check formatting
echo "6. Checking code formatting..."
cargo fmt --check -p driver-dover-motion -p common
echo "✓ Code formatting correct"
echo ""

# 7. Run clippy lints
echo "7. Running clippy lints..."
cargo clippy -p driver-dover-motion -- -D warnings
cargo clippy -p common --lib -- -D warnings
echo "✓ No clippy warnings"
echo ""

# 8. Verify workspace includes new crate
echo "8. Verifying workspace configuration..."
if grep -q "driver-dover-motion" Cargo.toml; then
    echo "✓ driver-dover-motion is in workspace members"
else
    echo "✗ driver-dover-motion NOT in workspace members!"
    exit 1
fi
echo ""

# 9. Check that TriggerOnPosition trait exists
echo "9. Verifying TriggerOnPosition trait..."
if grep -q "pub trait TriggerOnPosition" crates/common/src/capabilities.rs; then
    echo "✓ TriggerOnPosition trait found in capabilities.rs"
else
    echo "✗ TriggerOnPosition trait NOT found!"
    exit 1
fi
echo ""

# 10. Check that TriggerOnPosition capability exists
echo "10. Verifying TriggerOnPosition capability..."
if grep -q "TriggerOnPosition," crates/common/src/driver.rs; then
    echo "✓ TriggerOnPosition capability found in driver.rs"
else
    echo "✗ TriggerOnPosition capability NOT found!"
    exit 1
fi
echo ""

echo "=== ALL VERIFICATIONS PASSED ==="
echo ""
echo "Next steps:"
echo "1. Review COMPLETION_CHECKLIST.md for manual steps"
echo "2. Commit changes: git add -A && git commit -m '...'"
echo "3. Push to remote: git push origin bd-bd-3yb8.20"
echo "4. Update bead status: bd update bd-3yb8.2 --status inreview"
echo ""
