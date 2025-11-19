#!/bin/bash

################################################################################
# V4 Hardware Testing - Executable Scripts & Templates
#
# Use these scripts on maitai-eos to prepare and execute hardware tests
# Location: ~/rust-daq/v4-daq/docs/testing/HARDWARE_TESTING_SCRIPTS.sh
#
# Copy to maitai-eos and execute:
#   bash prepare_hardware_testing.sh
#   bash run_hardware_tests.sh
################################################################################

# ============================================================================
# SCRIPT 1: prepare_hardware_testing.sh
#
# Prepares environment for hardware testing (run first)
# Time: 10 minutes
# ============================================================================

cat > prepare_hardware_testing.sh << 'PREPARE_SCRIPT'
#!/bin/bash

set -e

echo "=================================================="
echo "V4 Hardware Testing - Environment Preparation"
echo "=================================================="
echo "Timestamp: $(date)"
echo ""

# Step 1: Create results directory
RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"
mkdir -p "$RESULTS_DIR"
echo "[1/7] Created results directory: $RESULTS_DIR"

# Step 2: Verify project exists
cd ~/rust-daq/v4-daq
if [ ! -f Cargo.toml ]; then
  echo "ERROR: Cargo.toml not found in $(pwd)"
  exit 1
fi
echo "[2/7] Verified V4 project structure"

# Step 3: Set environment variables
export RUST_LOG=debug
export RUST_BACKTRACE=full
echo "[3/7] Set environment variables"
echo "    RUST_LOG=$RUST_LOG"
echo "    RUST_BACKTRACE=$RUST_BACKTRACE"

# Step 4: Verify Rust toolchain
RUST_VERSION=$(rustc --version)
CARGO_VERSION=$(cargo --version)
echo "[4/7] Rust toolchain verified"
echo "    $RUST_VERSION"
echo "    $CARGO_VERSION"

# Step 5: Build all examples
echo "[5/7] Building hardware test examples..."
cargo build --examples --release 2>&1 | grep -E "Compiling|Finished|error" || echo "Build in progress..."

# Step 6: Record hardware configuration baseline
echo "[6/7] Recording hardware baseline..."
cat > "$RESULTS_DIR/hardware_baseline.txt" << 'BASELINE'
V4 Hardware Test Configuration Baseline
========================================
Date: $(date)
Location: maitai-eos
Operator: [Name]

HARDWARE IDENTIFICATION:
SCPI Instrument (generic): [VISA resource from visainfo]
Newport 1830-C: [Serial port]
MaiTai Ti:Sapphire: [Serial port]
ESP300 Motion Controller: [Serial port]
PVCAM Camera: [Camera model/serial]

ENVIRONMENT CONDITIONS:
Lab Temperature: [°C]
Humidity: [%]
Power Supply: [Stable/UPS]
Network: [Tailscale VPN]

BASELINE MEASUREMENTS:
SCPI IDN: [Response string]
Newport Power: [Power reading in dark] mW
MaiTai Wavelength: [Default wavelength] nm
MaiTai Power: [Power output] mW
ESP300 Position: [X, Y, Z] mm
PVCAM Temp: [Sensor temperature] °C

CHECKLIST:
☐ All instruments warmed up (15 min minimum)
☐ All serial cables physically connected
☐ Network connections verified
☐ USB devices recognized
☐ Lab environment stable
☐ Safety equipment accessible
☐ Emergency contacts known
☐ Test procedures reviewed
BASELINE

echo "Baseline saved to: $RESULTS_DIR/hardware_baseline.txt"

# Step 7: Create test logging utilities
echo "[7/7] Creating test utilities..."

cat > test_logger.sh << 'LOGGER'
#!/bin/bash
# Logs all test output with timestamp

TEST_NAME=$1
RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"
LOG_FILE="$RESULTS_DIR/${TEST_NAME}_$(date +%s).log"

{
  echo "=========================================="
  echo "Test: $TEST_NAME"
  echo "Started: $(date)"
  echo "=========================================="
  echo ""

  shift
  "$@"

  echo ""
  echo "=========================================="
  echo "Completed: $(date)"
  echo "=========================================="
} 2>&1 | tee "$LOG_FILE"

echo ""
echo "Log saved to: $LOG_FILE"
LOGGER
chmod +x test_logger.sh

echo ""
echo "=================================================="
echo "Environment Ready for Hardware Testing"
echo "=================================================="
echo "Results directory: $RESULTS_DIR"
echo "Next: Run run_hardware_tests.sh"
echo "=================================================="
PREPARE_SCRIPT

chmod +x prepare_hardware_testing.sh

# ============================================================================
# SCRIPT 2: run_hardware_tests.sh
#
# Executes all hardware tests in proper order
# Time: 6-7 hours
# ============================================================================

cat > run_hardware_tests.sh << 'TESTS_SCRIPT'
#!/bin/bash

set -e

RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"
START_TIME=$(date)
FAILURES=()

# Ensure test logger exists
if [ ! -f test_logger.sh ]; then
  echo "ERROR: test_logger.sh not found"
  exit 1
fi

# Helper function to run test safely
run_test() {
  local test_name=$1
  local example=$2
  local env_var=$3

  echo ""
  echo "=========================================="
  echo "Starting: $test_name"
  echo "=========================================="

  # Set environment variable if provided
  if [ -n "$env_var" ]; then
    export $env_var
  fi

  # Run with timeout
  if timeout 300 cargo run --example "$example" --release 2>&1 | tee "$RESULTS_DIR/${test_name}.log"; then
    echo "✓ $test_name PASSED"
    return 0
  else
    echo "✗ $test_name FAILED or TIMED OUT"
    FAILURES+=("$test_name")
    return 1
  fi
}

# ========== MAIN TEST EXECUTION ==========

cd ~/rust-daq/v4-daq
export RUST_LOG=debug

echo "=========================================="
echo "V4 Hardware Tests - Full Execution"
echo "=========================================="
echo "Start Time: $START_TIME"
echo "Results Dir: $RESULTS_DIR"
echo "Operator: [Your Name]"
echo "=========================================="
echo ""

# DAY 1: Low Risk Tests
echo "PHASE 1: Low-Risk Instruments"
echo "Expected Duration: 40 minutes"
echo ""

# Test 1: SCPI
export SCPI_RESOURCE="GPIB0::10::INSTR"  # UPDATE THIS
run_test "SCPI_Actor" "v4_scpi_hardware_test" || true

# Test 2: Newport
export NEWPORT_PORT="/dev/ttyUSB0"  # UPDATE THIS
run_test "Newport_1830C" "v4_newport_hardware_test" || true

# DAY 2: Medium Risk Tests
echo ""
echo "PHASE 2: Medium-Risk Instruments"
echo "Expected Duration: 1 hour 15 minutes"
echo ""

# Test 3: PVCAM
run_test "PVCAM_Camera" "v4_pvcam_hardware_test" || true

# Test 4: ESP300 (requires safety review)
echo ""
echo "⚠ SAFETY CHECK FOR ESP300 MOTION TESTING:"
read -p "Have you verified safe motion conditions? (y/n) " -n 1 esp_proceed
echo ""

if [ "$esp_proceed" = "y" ]; then
  export ESP300_PORT="/dev/ttyUSB0"  # UPDATE THIS
  run_test "ESP300_Motion" "v4_esp300_hardware_test" || true
else
  echo "ESP300 tests SKIPPED"
  FAILURES+=("ESP300_Motion (skipped)")
fi

# DAY 3: Critical Risk Test
echo ""
echo "PHASE 3: Critical-Risk Laser Testing"
echo "Expected Duration: 1.5 hours"
echo "⚠⚠ LASER SAFETY OFFICER REQUIRED ⚠⚠"
echo ""

read -p "Is Laser Safety Officer present and has approved testing? (y/n) " -n 1 laser_proceed
echo ""

if [ "$laser_proceed" = "y" ]; then
  export MAITAI_PORT="/dev/ttyUSB5"  # UPDATE THIS
  run_test "MaiTai_Laser" "v4_maitai_hardware_test" || true
else
  echo "MaiTai tests SKIPPED - safety approval not obtained"
  FAILURES+=("MaiTai_Laser (skipped)")
fi

# ========== RESULTS SUMMARY ==========

echo ""
echo "=========================================="
echo "Hardware Tests Completed"
echo "=========================================="
echo "End Time: $(date)"
echo "Results Dir: $RESULTS_DIR"
echo ""

if [ ${#FAILURES[@]} -eq 0 ]; then
  echo "✓ ALL TESTS PASSED"
  exit 0
else
  echo "✗ TESTS WITH ISSUES:"
  for failure in "${FAILURES[@]}"; do
    echo "  - $failure"
  done
  exit 1
fi
TESTS_SCRIPT

chmod +x run_hardware_tests.sh

# ============================================================================
# SCRIPT 3: verify_hardware.sh
#
# Pre-test hardware verification (run before testing starts)
# Time: 5 minutes
# ============================================================================

cat > verify_hardware.sh << 'VERIFY_SCRIPT'
#!/bin/bash

echo "============================================"
echo "V4 Hardware Verification"
echo "============================================"
echo "Timestamp: $(date)"
echo ""

ISSUES=0

# Check 1: SCPI/VISA
echo "[1/5] VISA Library & SCPI Instruments"
if which visainfo > /dev/null 2>&1; then
  VISA_COUNT=$(visainfo 2>/dev/null | grep -c "Resource Class" || echo 0)
  if [ $VISA_COUNT -gt 0 ]; then
    echo "  ✓ VISA installed with $VISA_COUNT resources"
    visainfo 2>/dev/null | grep "Resource Class" | head -3
  else
    echo "  ✗ WARNING: No VISA resources found"
    echo "    Run: visainfo"
    ((ISSUES++))
  fi
else
  echo "  ✗ VISA not installed"
  echo "    Install: sudo apt install ni-visa"
  ((ISSUES++))
fi
echo ""

# Check 2: Serial Ports
echo "[2/5] Serial Port Availability"
SERIAL_PORTS=$(ls /dev/ttyUSB* 2>/dev/null | wc -l)
if [ $SERIAL_PORTS -gt 0 ]; then
  echo "  ✓ Found $SERIAL_PORTS USB serial ports:"
  ls -la /dev/ttyUSB* 2>/dev/null | awk '{print "    " $NF}'
else
  echo "  ✗ No USB serial ports found"
  echo "    Check: Instruments powered on and connected"
  ((ISSUES++))
fi
echo ""

# Check 3: PVCAM Camera
echo "[3/5] PVCAM Camera"
if lsusb 2>/dev/null | grep -qi photometric; then
  echo "  ✓ Camera detected:"
  lsusb | grep -i photometric
else
  echo "  ✗ Camera NOT detected"
  echo "    Check: USB connection and power"
  ((ISSUES++))
fi
echo ""

# Check 4: Permissions
echo "[4/5] User Permissions"
USER_GROUPS=$(groups 2>/dev/null)
if echo "$USER_GROUPS" | grep -q "dialout"; then
  echo "  ✓ User in dialout group"
else
  echo "  ✗ User NOT in dialout group"
  echo "    Fix: sudo usermod -a -G dialout \$USER && logout"
  ((ISSUES++))
fi
echo ""

# Check 5: Disk Space
echo "[5/5] Disk Space"
DISK_AVAILABLE=$(df -B1 ~/rust-daq/v4-daq | awk 'NR==2 {print $4}')
DISK_GB=$((DISK_AVAILABLE / 1024 / 1024 / 1024))
if [ $DISK_GB -gt 1 ]; then
  echo "  ✓ Adequate disk space: ${DISK_GB} GB available"
else
  echo "  ✗ Low disk space: ${DISK_GB} GB available"
  echo "    Need: At least 1 GB for test logs"
  ((ISSUES++))
fi
echo ""

# Summary
echo "============================================"
if [ $ISSUES -eq 0 ]; then
  echo "✓ All hardware verified - Ready to test"
  exit 0
else
  echo "✗ $ISSUES issues found - Address before testing"
  exit 1
fi
VERIFY_SCRIPT

chmod +x verify_hardware.sh

# ============================================================================
# SCRIPT 4: post_test_analysis.sh
#
# Analyzes results after testing completes
# Time: 10 minutes
# ============================================================================

cat > post_test_analysis.sh << 'ANALYSIS_SCRIPT'
#!/bin/bash

RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"

if [ ! -d "$RESULTS_DIR" ]; then
  echo "ERROR: Results directory not found: $RESULTS_DIR"
  exit 1
fi

echo "=========================================="
echo "Hardware Test Results Analysis"
echo "=========================================="
echo "Results Directory: $RESULTS_DIR"
echo ""

# Generate summary
echo "=== Test Summary ===" > "$RESULTS_DIR/RESULTS_SUMMARY.txt"
echo "Date: $(date)" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"
echo "" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"

TOTAL_TESTS=0
PASSED_TESTS=0

for logfile in "$RESULTS_DIR"/*.log; do
  if [ ! -f "$logfile" ]; then
    continue
  fi

  TEST_NAME=$(basename "$logfile" .log)
  echo "" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"
  echo "=== $TEST_NAME ===" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"

  # Count test results
  if grep -q "panic" "$logfile"; then
    echo "Status: PANIC - Actor crashed" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"
    ((TOTAL_TESTS++))
  elif grep -q "ERROR" "$logfile"; then
    echo "Status: FAILED - Errors in log" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"
    ((TOTAL_TESTS++))
    grep "ERROR" "$logfile" | head -3 >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"
  else
    echo "Status: PASSED" >> "$RESULTS_DIR/RESULTS_SUMMARY.txt"
    ((PASSED_TESTS++))
    ((TOTAL_TESTS++))
  fi
done

# Print summary
echo ""
echo "Test Results Summary:"
cat "$RESULTS_DIR/RESULTS_SUMMARY.txt"

# Regression check
echo ""
echo "Checking for regressions..."
cd ~/rust-daq/v4-daq
if cargo test --test integration_actors_test --release 2>&1 | tee "$RESULTS_DIR/regression_test.log" | grep -q "test result: ok"; then
  echo "✓ No regressions - all mock tests still pass"
else
  echo "✗ WARNING: Some mock tests may have failed"
fi

# Final verdict
echo ""
echo "=========================================="
if [ $PASSED_TESTS -eq $TOTAL_TESTS ]; then
  echo "✓ HARDWARE VALIDATION SUCCESSFUL"
  echo "  All tests passed. Ready for production."
else
  echo "✗ HARDWARE VALIDATION INCOMPLETE"
  echo "  $PASSED_TESTS/$TOTAL_TESTS tests passed"
  echo "  Review logs for failures"
fi
echo "=========================================="
ANALYSIS_SCRIPT

chmod +x post_test_analysis.sh

# ============================================================================
# FINAL SUMMARY
# ============================================================================

echo ""
echo "=================================================="
echo "Hardware Testing Scripts Created"
echo "=================================================="
echo ""
echo "Scripts available in current directory:"
echo "  1. prepare_hardware_testing.sh  - Setup environment (run first)"
echo "  2. run_hardware_tests.sh        - Execute all tests"
echo "  3. verify_hardware.sh           - Pre-test verification"
echo "  4. post_test_analysis.sh        - Analyze results"
echo "  5. test_logger.sh               - Created by prepare script"
echo ""
echo "Execution order:"
echo "  1. bash prepare_hardware_testing.sh"
echo "  2. bash verify_hardware.sh"
echo "  3. bash run_hardware_tests.sh"
echo "  4. bash post_test_analysis.sh"
echo ""
echo "=================================================="
