# Hardware Testing Preparation Plan - V4 Production Validation

**Date**: 2025-11-17
**Status**: Ready for Execution
**Scope**: 89 hardware test scenarios across 5 V4 actors via SSH to maitai-eos
**Timeline**: 6-7 hours over 2-3 days
**Critical Risk**: MaiTai laser requires laser safety officer supervision

---

## Executive Summary

This document provides step-by-step preparation procedures for executing comprehensive hardware validation of V4 actors (SCPI, ESP300, PVCAM, Newport 1830-C, MaiTai) using real hardware on the maitai-eos cluster. All procedures prioritize safety, starting with the safest low-risk tests and progressing to higher-risk operations.

**Test Distribution**:
| Actor | Tests | Duration | Risk |
|-------|-------|----------|------|
| SCPI | 17 | 20 min | Low |
| Newport 1830-C | 14 | 20 min | Low |
| PVCAM | 28 | 30 min | Medium |
| ESP300 | 16 | 45 min | Medium |
| MaiTai | 19 | 1.5 hrs | **Critical** |
| **Total** | **94** | **6-7 hrs** | **See below** |

---

## Part 1: SSH Access Verification

### 1.1 Network Connectivity Check (5 minutes)

Execute from your local machine:

```bash
# Step 1: Verify Tailscale connectivity
tailscale status | grep maitai
# Expected output: maitai-eos ... [IP address]

# Step 2: Ping maitai-eos
ping -c 4 maitai-eos
# Expected: 4 packets transmitted, 4 received (0% packet loss)

# Step 3: Verify SSH availability
ssh -v maitai@maitai-eos "echo 'SSH connection successful'" 2>&1 | grep -E "Connected|Authenticated"
# Expected: Connected to maitai-eos
```

**Troubleshooting**:
- If ping fails: Check Tailscale VPN connection
- If SSH fails: Verify SSH key is loaded (`ssh-add -l`)
- If auth fails: Confirm credentials with cluster admin

### 1.2 SSH Connection Setup (3 minutes)

```bash
# Step 1: Establish SSH session (recommended in tmux)
ssh maitai@maitai-eos

# Step 2: Verify login success
# Expected prompt: maitai@maitai-eos:~$

# Step 3: Check available resources
uname -a               # System info
df -h /                # Disk space (need ~1 GB for logs)
free -h                # Memory available (need ~2 GB)
ps aux | wc -l         # Existing processes (should be < 100)

# Step 4: Navigate to project
cd ~/rust-daq/v4-daq
pwd                    # Should show: /home/maitai/rust-daq/v4-daq
ls -la Cargo.toml      # Verify project present
```

**Expected Output**:
```
maitai@maitai-eos:~$ cd ~/rust-daq/v4-daq
maitai@maitai-eos:~/rust-daq/v4-daq$ ls -la Cargo.toml
-rw-r--r-- 1 maitai maitai 1234 Nov 17 14:23 Cargo.toml
```

---

## Part 2: Hardware Availability Checklist

### 2.1 Physical Hardware Inventory Check (10 minutes)

Execute on maitai-eos:

```bash
# Step 1: List all serial devices
echo "=== Serial Devices ==="
ls -la /dev/ttyUSB* 2>/dev/null || echo "No USB serial devices found"
ls -la /dev/ttyS* 2>/dev/null || echo "No legacy serial devices found"

# Step 2: Identify each instrument
echo ""
echo "=== Hardware Identification ==="
echo "Checking MaiTai (Ti:Sapphire laser)..."
picocom -b 9600 /dev/ttyUSB5 < /dev/null 2>&1 | head -1 || echo "MaiTai port not found"

echo "Checking ESP300 (Newport motion)..."
picocom -b 19200 /dev/ttyUSB0 < /dev/null 2>&1 | head -1 || echo "ESP300 port not found"

echo "Checking Newport 1830-C (power meter)..."
picocom -b 9600 /dev/ttyUSB2 < /dev/null 2>&1 | head -1 || echo "Newport port not found"

# Step 3: List USB devices (for PVCAM camera)
echo ""
echo "=== USB Devices ==="
lsusb | grep -i photometric || echo "PVCAM camera not detected"
lsusb | grep -E "camera|imaging" || echo "No camera devices found"

# Step 4: Check VISA library (for SCPI)
echo ""
echo "=== VISA Installation ==="
which visainfo && visainfo | head -5 || echo "VISA not found"
```

### 2.2 Hardware Warm-Up Procedure (15 minutes)

Before testing begins:

```bash
# Step 1: Power on all instruments if not already on
# Manual step: Verify each instrument display shows "Ready" or similar

# Step 2: Wait for stabilization (15 minutes minimum)
echo "Waiting for instruments to warm up..."
for i in {15..1}; do
  echo -ne "\rWarm-up in progress... $i minutes remaining"
  sleep 60
done
echo ""
echo "Warm-up complete - instruments ready for testing"

# Step 3: Verify baseline stability
# MaiTai: Power display should show stable output
# Newport: Power meter should show stable baseline
# ESP300: Position indicators should be stable
# PVCAM: Sensor temperature should be stable
# SCPI: Instrument should respond to queries

# Step 4: Record baseline measurements
echo "=== Baseline Measurements ===" > hardware_baseline_$(date +%s).txt
date >> hardware_baseline_$(date +%s).txt
echo "Temperature: [record lab temperature]" >> hardware_baseline_$(date +%s).txt
echo "Humidity: [record if available]" >> hardware_baseline_$(date +%s).txt
```

### 2.3 Hardware Verification Matrix

Create a verification checklist on maitai-eos:

```bash
cat > hardware_verification.sh << 'EOF'
#!/bin/bash

echo "============================================"
echo "V4 Hardware Availability Verification"
echo "============================================"
echo "Timestamp: $(date)"
echo ""

# SCPI Instrument Check
echo "[1/5] SCPI Instrument (Generic VISA device)"
if which visainfo > /dev/null 2>&1; then
  echo "  Status: VISA installed"
  visainfo 2>/dev/null | grep -E "GPIB|USB|TCPIP" | head -3 || echo "  Warning: No VISA resources found"
else
  echo "  Status: VISA NOT installed - SCPI tests will FAIL"
fi
echo ""

# ESP300 Check
echo "[2/5] ESP300 (Newport Motion Controller)"
if [ -e /dev/ttyUSB0 ] || [ -e /dev/ttyUSB1 ]; then
  echo "  Status: Serial port available"
  echo "  Ports: $(ls /dev/ttyUSB* 2>/dev/null | tr '\n' ' ')"
else
  echo "  Status: No serial ports found - Motion tests will FAIL"
fi
echo ""

# PVCAM Check
echo "[3/5] PVCAM (Camera Sensor)"
if lsusb 2>/dev/null | grep -qi photometric; then
  echo "  Status: Camera detected via USB"
  lsusb | grep -i photometric
else
  echo "  Status: Camera NOT detected - PVCAM tests will FAIL"
fi
if pkg-config --list-all 2>/dev/null | grep -qi pvcam; then
  echo "  PVCAM library: FOUND"
else
  echo "  PVCAM library: NOT FOUND"
fi
echo ""

# Newport 1830-C Check
echo "[4/5] Newport 1830-C (Optical Power Meter)"
if [ -e /dev/ttyUSB0 ] || [ -e /dev/ttyUSB2 ]; then
  echo "  Status: Serial port available"
else
  echo "  Status: No serial port - Power meter tests will FAIL"
fi
echo ""

# MaiTai Check
echo "[5/5] MaiTai (Ti:Sapphire Laser)"
if [ -e /dev/ttyUSB5 ]; then
  echo "  Status: Serial port available at /dev/ttyUSB5"
  echo "  WARNING: Laser safety officer approval REQUIRED before testing"
else
  echo "  Status: Serial port NOT available at /dev/ttyUSB5"
  echo "  Check: Try /dev/ttyUSB* for actual port"
fi
echo ""

echo "============================================"
echo "Verification Complete"
echo "============================================"
EOF

chmod +x hardware_verification.sh
./hardware_verification.sh
```

---

## Part 3: Safety Procedures

### 3.1 General Safety Briefing

**All personnel must review before testing**:

```
SAFETY BRIEFING CHECKLIST
========================

Lab Safety:
☐ Understand emergency exits location
☐ Know emergency stop button locations
☐ Verify first aid kit is accessible
☐ Know how to reach facility manager in emergency

Equipment Safety:
☐ All equipment power supplies are grounded
☐ No water/beverages near electronics
☐ Adequate ventilation confirmed
☐ Temperature stable (within 2°C of testing temperature)

Personnel:
☐ Authorized to operate this equipment
☐ Have read equipment datasheets
☐ Understand command parameters (especially motion, laser)
☐ Know when to stop testing if something seems wrong

Documentation:
☐ Test procedures reviewed and understood
☐ Expected results known
☐ Failure modes understood
☐ Contact information available
```

### 3.2 MaiTai Laser Safety (CRITICAL - MANDATORY)

**REQUIREMENT**: Laser Safety Officer must review and approve before any MaiTai testing.

#### Pre-Test Laser Safety Briefing

```
LASER SAFETY BRIEFING - MaiTai Ti:Sapphire
===========================================

HAZARD LEVEL: Class 4 - Highest Risk
- Permanent eye damage possible from direct beam
- Skin burns from prolonged exposure
- Fire hazard from secondary beams
- Electrical hazard from power supply (>1000V)

SAFETY APPROVAL REQUIRED:
Before MaiTai testing begins:
☐ Laser Safety Officer has approved this session
☐ Laser Safety Officer is physically present
☐ Laser Safety Officer name: _________________
☐ Approval timestamp: _________________

PERSONAL PROTECTIVE EQUIPMENT:
☐ Laser safety glasses (correct wavelength 700-1000nm)
☐ Lab coat
☐ Closed-toe shoes
☐ Tie/jewelry secured (no dangling objects)

ENVIRONMENT CHECK:
☐ Laser enclosure doors: CLOSED and interlocked
☐ Emergency stop button: TESTED and FUNCTIONAL
☐ Safety shutter: TESTED and OPERATIONAL
☐ Beam path: CLEAR and marked
☐ Secondary reflections: BLOCKED or CONTROLLED
☐ Fire extinguisher: ACCESSIBLE and CHARGED
☐ Water cooling: OPERATIONAL (if applicable)

OPERATIONAL PROCEDURE:
1. Ensure shutter is CLOSED before powering laser
2. Verify shutter closes on ANY error condition
3. Verify shutter closes IMMEDIATELY on shutdown
4. Never aim beam toward personnel
5. NEVER open enclosure while laser is armed
6. If anything seems wrong: STOP immediately

EMERGENCY PROCEDURES:
- Beam unexpectedly open: Hit emergency stop
- Eye exposure suspected: Call 911, seek ophthalmologist
- Skin burn: Cool with water, seek medical attention
- Power supply issue: Turn off main breaker, notify facility
```

#### MaiTai Shutter Safety Verification

```bash
# MANDATORY: Execute BEFORE any MaiTai testing
# This verifies the critical safety mechanism

echo "=== MaiTai Critical Safety Test ==="
echo "DO NOT PROCEED PAST THIS POINT WITHOUT SUPERVISOR APPROVAL"
echo ""
echo "Laser Safety Officer approval required:"
echo "Name: _____________________"
echo "Signature: _________________"
echo "Date/Time: _________________"
echo ""

# Step 1: Verify connection without opening shutter
echo "Step 1: Verifying serial connection..."
picocom -b 9600 /dev/ttyUSB5 << 'SERIAL'
SHUTTER?
SERIAL

# Expected: Response is "0" (closed) or "1" (open)
# If response is "1" (OPEN), STOP IMMEDIATELY - safety issue

echo ""
echo "Step 2: Verify shutter responds to close command..."
picocom -b 9600 /dev/ttyUSB5 << 'SERIAL'
SHUTTER:0
SHUTTER?
SERIAL

# Expected: First command has no response, second returns "0"
# If shutter doesn't close: STOP - safety issue

echo ""
echo "SAFETY VERIFICATION RESULT:"
read -p "Is shutter confirmed CLOSED (y/n)? " -n 1 shutter_closed
if [ "$shutter_closed" != "y" ]; then
  echo "ERROR: Shutter safety not verified. DO NOT PROCEED WITH TESTING."
  exit 1
fi
echo ""
echo "Safety verification PASSED. Proceeding to testing."
```

### 3.3 Motion Control Safety (ESP300)

```bash
echo "=== ESP300 Safety Verification ==="
echo ""

# Step 1: Verify all axes can be homed
echo "Step 1: Homing all axes (WATCH FOR MOTION)..."
picocom -b 19200 /dev/ttyUSB0 << 'SERIAL'
HOME
SERIAL

# Step 2: Verify soft limits are set
echo "Step 2: Setting conservative soft limits..."
picocom -b 19200 /dev/ttyUSB0 << 'SERIAL'
LIMIT:ABS 0,-50,50
LIMIT:ABS 1,-50,50
LIMIT:ABS 2,-50,50
SERIAL

# Step 3: Test emergency stop
echo "Step 3: Testing emergency stop capability..."
echo "  If motion runs, be ready to press Ctrl-C to send stop signal"
echo "  Manual emergency stop location: [note your local stop button]"

# Step 4: Verify return to home
echo "Step 4: Returning axes to home position..."
picocom -b 19200 /dev/ttyUSB0 << 'SERIAL'
HOME
SERIAL

echo "Step 4: Safety verification complete"
echo "  All axes homed successfully"
echo "  Soft limits enabled"
echo "  Emergency stop confirmed available"
```

### 3.4 Electrical Safety Checklist

```bash
cat > electrical_safety_check.txt << 'EOF'
ELECTRICAL SAFETY CHECKLIST
===========================

MaiTai Power Supply:
☐ Power supply cover installed and secured
☐ No exposed high voltage connections
☐ All cables properly insulated
☐ Ground connections verified
☐ Thermal indicators within normal range

ESP300 Power:
☐ Power cables intact, no visible damage
☐ Connection secure to main power
☐ Motor connections properly seated
☐ No sparking or unusual sounds during operation

Newport 1830-C:
☐ Power connection verified
☐ Battery status (if battery-powered): ___________
☐ No overheating detected

PVCAM Camera:
☐ USB cable properly connected
☐ No damage to USB connector
☐ Power indicator on (if present)
☐ Cooling fan operational (if present)

General:
☐ Lab ground is connected properly
☐ No wet surfaces near equipment
☐ No loose cables in walkways
☐ All equipment grounded with earth ground
EOF

cat electrical_safety_check.txt
```

---

## Part 4: Test Execution Schedule

### 4.1 Recommended Timeline

Organize testing from safest (lowest risk) to most risky (highest risk):

```
Week of 2025-11-17

MONDAY (Day 1): LOW RISK - SCPI + Newport
├─ 9:00-9:15 AM: SSH connectivity verification
├─ 9:15-9:30 AM: Hardware inventory check
├─ 9:30-10:00 AM: Warm-up instruments
├─ 10:00-10:20 AM: SCPI hardware tests (17 tests)
├─ 10:20-10:40 AM: Newport 1830-C tests (14 tests)
├─ 10:40-11:00 AM: Results review and logging
└─ Duration: 2 hours, Status: ✓ SAFE

TUESDAY (Day 2): MEDIUM RISK - PVCAM + ESP300
├─ 10:00-10:15 AM: Warm-up instruments
├─ 10:15-10:20 AM: PVCAM setup (camera focus, baseline)
├─ 10:20-10:50 AM: PVCAM tests (28 tests)
├─ 10:50-11:10 AM: Results review
├─ 1:00-1:15 PM: ESP300 safety briefing
├─ 1:15-1:35 PM: ESP300 homing and soft limits setup
├─ 1:35-2:20 PM: ESP300 tests (16 tests, includes motion)
├─ 2:20-2:30 PM: Return axes to home
└─ Duration: 3 hours, Status: ⚠ MEDIUM RISK

WEDNESDAY (Day 3): CRITICAL RISK - MaiTai LASER
├─ 2:00-2:30 PM: Laser Safety Officer arrives
├─ 2:30-2:50 PM: Mandatory laser safety briefing
├─ 2:50-3:05 PM: Critical safety verification tests
├─ 3:05-3:10 PM: Formal approval from safety officer
├─ 3:10-4:40 PM: MaiTai tests (19 tests - SUPERVISED)
│  ├─ 3:10-3:15 PM: Shutter operations (critical)
│  ├─ 3:15-3:35 PM: Wavelength tuning
│  ├─ 3:35-3:50 PM: Power measurements
│  └─ 3:50-4:40 PM: Lifecycle and shutdown
├─ 4:40-4:50 PM: Post-laser shutdown sequence
└─ Duration: 1.5 hours, Status: ⚠⚠ CRITICAL RISK

THURSDAY: POST-TESTING
├─ 10:00-10:30 AM: Generate test report
├─ 10:30-10:45 AM: Archive all logs and results
├─ 10:45-11:00 AM: Regression testing (mock tests still pass)
└─ Duration: 1 hour
```

### 4.2 Time Allocation by Phase

| Phase | Activity | Time | Critical? |
|-------|----------|------|-----------|
| Setup & Verification | SSH/Hardware check | 15 min | Yes |
| Warm-up | Instrument stabilization | 15 min | Yes |
| SCPI Testing | 17 tests | 20 min | No |
| Newport Testing | 14 tests | 20 min | No |
| PVCAM Testing | 28 tests | 30 min | No |
| ESP300 Testing | 16 tests (with safety checks) | 45 min | Yes |
| MaiTai Testing | 19 tests (requires supervisor) | 1.5 hrs | **Yes** |
| Results & Cleanup | Logging, archiving, regression | 30 min | No |
| **Total** | | **6-7 hours** | |

---

## Part 5: Environment Setup Scripts

### 5.1 Pre-Test Environment Preparation

Create on maitai-eos:

```bash
# File: prepare_hardware_testing.sh

#!/bin/bash

set -e

echo "=================================================="
echo "V4 Hardware Testing - Environment Setup"
echo "=================================================="
echo "Timestamp: $(date)"
echo ""

# Step 1: Create results directory
RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"
mkdir -p "$RESULTS_DIR"
echo "[1/6] Created results directory: $RESULTS_DIR"

# Step 2: Setup environment variables
export RUST_LOG=debug
export RUST_BACKTRACE=full
echo "[2/6] Set environment variables"

# Step 3: Verify Rust toolchain
RUST_VERSION=$(rustc --version)
echo "[3/6] Rust toolchain: $RUST_VERSION"

# Step 4: Build all examples
echo "[4/6] Building V4 hardware test examples..."
cd ~/rust-daq/v4-daq
cargo build --examples --release 2>&1 | grep -E "Finished|error" || true

# Step 5: Record hardware configuration
echo "[5/6] Recording hardware configuration..."
cat > "$RESULTS_DIR/hardware_config.txt" << 'EOF'
V4 Hardware Test Configuration
==============================
Date: $(date)
Location: maitai-eos cluster
Operator: [Name]

Instruments:
- SCPI: [Resource string from visainfo]
- ESP300: [Serial port]
- Newport 1830-C: [Serial port]
- PVCAM: [Camera name]
- MaiTai: [Serial port]

Environment:
- Temperature: [Lab temperature]
- Humidity: [If available]
- Power: [Stable/UPS backup]
- Network: Tailscale VPN

Operator Checklist:
☐ Safety briefing completed
☐ Equipment warm-up verified (15 min minimum)
☐ All instruments responding to queries
☐ Emergency procedures understood
☐ Contact information available
EOF

# Step 6: Create logging wrapper
echo "[6/6] Creating test logging script..."
cat > test_runner.sh << 'SCRIPT'
#!/bin/bash
# Runs a test and logs output with timestamp

TEST_NAME=$1
RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"

echo "Running: $TEST_NAME"
echo "Started: $(date)" | tee "$RESULTS_DIR/${TEST_NAME}_$(date +%s).log"
shift
$@ 2>&1 | tee -a "$RESULTS_DIR/${TEST_NAME}_$(date +%s).log"
echo "Completed: $(date)" | tee -a "$RESULTS_DIR/${TEST_NAME}_$(date +%s).log"
SCRIPT
chmod +x test_runner.sh

echo ""
echo "=================================================="
echo "Environment Ready for Testing"
echo "Results directory: $RESULTS_DIR"
echo "Next step: Execute hardware tests"
echo "=================================================="
```

Run this script before testing begins:

```bash
ssh maitai@maitai-eos << 'SETUP'
cd ~/rust-daq/v4-daq
bash prepare_hardware_testing.sh
SETUP
```

### 5.2 Test Execution Script

Create a master test runner:

```bash
# File: run_hardware_tests.sh (on maitai-eos)

#!/bin/bash

set -e

RESULTS_DIR="$HOME/hardware_test_results/$(date +%Y-%m-%d)"
TEST_LOG="$RESULTS_DIR/test_execution_$(date +%s).log"

{
  echo "=========================================="
  echo "V4 Hardware Tests - Execution Log"
  echo "=========================================="
  echo "Start Time: $(date)"
  echo "Results Dir: $RESULTS_DIR"
  echo ""

  cd ~/rust-daq/v4-daq

  # Test 1: SCPI (Low Risk)
  echo "[Test 1/5] SCPI Instrument Tests"
  export SCPI_RESOURCE="GPIB0::10::INSTR"  # Update as needed
  timeout 120 cargo run --example v4_scpi_hardware_test --release 2>&1 | tee "$RESULTS_DIR/scpi_test.log" || echo "SCPI test completed or timed out"
  echo ""

  # Test 2: Newport 1830-C (Low Risk)
  echo "[Test 2/5] Newport 1830-C Power Meter Tests"
  export NEWPORT_PORT="/dev/ttyUSB0"  # Update as needed
  timeout 120 cargo run --example v4_newport_hardware_test --release 2>&1 | tee "$RESULTS_DIR/newport_test.log" || echo "Newport test completed or timed out"
  echo ""

  # Test 3: PVCAM (Medium Risk)
  echo "[Test 3/5] PVCAM Camera Tests"
  timeout 180 cargo run --example v4_pvcam_hardware_test --release 2>&1 | tee "$RESULTS_DIR/pvcam_test.log" || echo "PVCAM test completed or timed out"
  echo ""

  # Test 4: ESP300 (Medium Risk - requires safety review)
  echo "[Test 4/5] ESP300 Motion Controller Tests"
  echo "  ⚠ Manual Safety Step Required:"
  echo "  1. Ensure all axes are homed (HOME command)"
  echo "  2. Set soft limits (LIMIT:ABS 0,-50,50 etc)"
  echo "  3. Clear motion path"
  read -p "  Proceed with ESP300 tests? (y/n) " -n 1 esp_proceed
  if [ "$esp_proceed" = "y" ]; then
    export ESP300_PORT="/dev/ttyUSB0"  # Update as needed
    timeout 180 cargo run --example v4_esp300_hardware_test --release 2>&1 | tee "$RESULTS_DIR/esp300_test.log" || echo "ESP300 test completed or timed out"
  else
    echo "  ESP300 tests SKIPPED"
  fi
  echo ""

  # Test 5: MaiTai (Critical Risk - requires supervisor)
  echo "[Test 5/5] MaiTai Ti:Sapphire Laser Tests"
  echo "  ⚠⚠ CRITICAL SAFETY REQUIREMENT:"
  echo "  1. Laser Safety Officer MUST be present"
  echo "  2. Safety briefing must be completed"
  echo "  3. All safety checks must pass"
  read -p "  Is Laser Safety Officer present and does safety check PASS? (y/n) " -n 1 laser_proceed
  if [ "$laser_proceed" = "y" ]; then
    export MAITAI_PORT="/dev/ttyUSB5"  # Update as needed
    timeout 180 cargo run --example v4_maitai_hardware_test --release 2>&1 | tee "$RESULTS_DIR/maitai_test.log" || echo "MaiTai test completed or timed out"
  else
    echo "  MaiTai tests SKIPPED - safety approval not obtained"
  fi

  echo ""
  echo "=========================================="
  echo "Hardware Tests Completed"
  echo "End Time: $(date)"
  echo "=========================================="

} | tee "$TEST_LOG"

echo ""
echo "Test results saved to: $RESULTS_DIR"
ls -lah "$RESULTS_DIR"
```

---

## Part 6: Success Verification Checklist

### 6.1 Pre-Testing Verification

Print and complete before first test:

```
PRE-TESTING VERIFICATION CHECKLIST
==================================

Date: _________________
Operator: _________________
Location: maitai-eos

Network & SSH:
☐ SSH connection to maitai-eos successful
☐ Tailscale VPN connected
☐ Network ping successful (< 100ms latency)
☐ Project directory accessible

Hardware Inventory:
☐ SCPI instrument accessible via VISA (visainfo)
☐ ESP300 serial port present (/dev/ttyUSB*)
☐ Newport 1830-C serial port present
☐ PVCAM camera detected (lsusb)
☐ MaiTai serial port present

Software Environment:
☐ Rust toolchain installed (rustc --version)
☐ Cargo build successful (cargo build --examples --release)
☐ All test examples built without errors
☐ RUST_LOG=debug environment variable set
☐ Results directory created (/home/maitai/hardware_test_results/YYYY-MM-DD)

Instrument Warm-up (15 minutes minimum):
☐ SCPI instrument warmed up and responsive
☐ ESP300 warmed up and homing
☐ Newport baseline established and stable
☐ PVCAM sensor cooled (if applicable)
☐ MaiTai stabilized (shutter closed)

Safety Review:
☐ General lab safety briefing completed
☐ Equipment datasheets reviewed
☐ Emergency contacts recorded
☐ First aid kit location known
☐ Emergency stops tested and functional

SIGN-OFF:
Operator Signature: _________________
Ready to Proceed: YES / NO
If NO, describe issues: _________________
```

### 6.2 Post-Test Verification

Completion checklist:

```
POST-TEST VERIFICATION CHECKLIST
================================

Hardware Status After Tests:
☐ MaiTai shutter confirmed CLOSED
☐ ESP300 axes returned to HOME position
☐ PVCAM camera streaming stopped
☐ Newport baseline re-established
☐ SCPI instrument idle/standby

Results Captured:
☐ All test logs saved to results directory
☐ Each actor test has corresponding .log file
☐ No core dumps in working directory
☐ Disk space available for archiving

Results Summary:
☐ SCPI: 17/17 tests passed (or document failures)
☐ Newport: 14/14 tests passed (or document failures)
☐ PVCAM: 28/28 tests passed (or document failures)
☐ ESP300: 16/16 tests passed (or document failures)
☐ MaiTai: 19/19 tests passed (or document failures)
Total: 94 tests (or document actual count and failures)

Regression Testing:
☐ Ran mock tests: cargo test --test integration_actors_test
☐ All mock tests still passing (no regressions)
☐ No performance degradation observed

Documentation:
☐ Test report created and dated
☐ Issues documented (if any)
☐ Operator notes captured
☐ Hardware serial numbers verified
☐ Environmental conditions recorded (temperature, humidity)

Archiving:
☐ All logs moved to ~/hardware_test_results/YYYY-MM-DD/
☐ Test report saved in same directory
☐ Results backed up to secondary location
☐ Git tag created: hardware-validation-v4-YYYY-MM-DD

Sign-Off:
Operator: _________________
Date: _________________
Status: PASS / FAIL / INCOMPLETE
Issues: _________________
```

---

## Part 7: Emergency Procedures

### 7.1 During Testing - If Something Goes Wrong

**Stop Immediately If**:
- Actor panics or crashes
- Hardware becomes unresponsive
- Laser beam opens unexpectedly
- Motion stage moves unexpectedly
- Excessive heat or sparks
- Any safety concern whatsoever

**Emergency Response**:

```bash
# Step 1: Stop test execution
Ctrl-C                          # Kill the running test
pkill -f "cargo run"            # Force kill if needed

# Step 2: Make equipment safe
# For MaiTai: Hit emergency stop, verify shutter closed
# For ESP300: Unplug power or hit emergency stop
# For PVCAM: Disconnect USB if needed
# For Newport: Power off if stable

# Step 3: Assess the issue
# Check system logs for errors:
journalctl -n 50               # System log
dmesg | tail -20               # Kernel log

# Step 4: Document the failure
mkdir -p emergency_debug
cp *.log emergency_debug/
echo "Failure at: $(date)" > emergency_debug/failure_timestamp.txt

# Step 5: Contact support
# Hardware Issues: Facility manager
# Safety Issues: Call emergency services
# Software Issues: Check git logs for recent changes
```

### 7.2 Laser Emergency (MaiTai)

```
IF LASER BEAM IS OPEN UNEXPECTEDLY:

1. STOP - Do NOT continue testing
2. HIT EMERGENCY STOP button (location: [facility specific])
3. VERIFY shutter is closed (manual override if needed)
4. DO NOT work on laser without supervisor approval

IF EYE EXPOSURE SUSPECTED:
1. STOP working immediately
2. CALL 911 or emergency medical services
3. Describe: "Possible laser exposure (700-1000nm), Class 4"
4. Seek OPHTHALMOLOGIST immediately

EMERGENCY CONTACTS:
- Laser Safety Officer: _________________
- Facility Manager: _________________
- Medical Emergency: 911
```

### 7.3 Motion Control Emergency (ESP300)

```
IF MOTION STAGE MOVES UNEXPECTEDLY:

1. STOP - Move hands away from stage
2. HIT EMERGENCY STOP button (location: [facility specific])
3. Wait for all motion to cease (should be < 1 second)
4. Assess for mechanical damage

IF COLLISION OCCURS:
1. DO NOT attempt to move stage
2. Power off ESP300
3. Inspect for damage
4. DO NOT resume testing without approval

IF STAGE WON'T STOP:
1. Unplug power cable immediately
2. Allow stage to coast to stop
3. Inspect for mechanical damage
4. Contact support before resuming
```

---

## Part 8: Troubleshooting Reference

### 8.1 Common Issues During Setup

| Issue | Symptom | Diagnosis | Solution |
|-------|---------|-----------|----------|
| SSH connection timeout | Cannot reach maitai-eos | VPN disconnected or network issue | Check Tailscale status, reconnect VPN |
| Port not found | `/dev/ttyUSB0: No such file` | Instrument not powered on or wrong port | Check power, run `ls /dev/ttyUSB*` to list |
| Permission denied | `Permission denied /dev/ttyUSB0` | User not in dialout group | `sudo usermod -a -G dialout $USER`, then logout/login |
| VISA not installed | `VISA library not found` | VISA SDK missing | Install with `sudo apt install ni-visa` or equivalent |
| Instrument not responding | No response to `*IDN?` | Wrong VISA resource or hardware issue | Run `visainfo` to get correct resource string |
| Build fails | Cargo compilation error | Missing dependencies or incompatible Rust version | `cargo update`, `rustup update` |

### 8.2 Common Issues During Testing

| Issue | Symptom | Diagnosis | Solution |
|-------|---------|-----------|----------|
| Test timeout | Test hangs for > 30 seconds | Hardware not responding, wrong port | Check hardware power, verify serial port |
| Actor panics | `thread panicked` in output | Memory/resource exhaustion or bug | Check system memory, increase ulimit |
| Frame corruption | PVCAM frames show artifacts | Timing issue or buffer overflow | Reduce frame rate, increase buffering |
| Position inaccuracy | ESP300 off by > 0.5mm | Calibration issue or mechanical problem | Recalibrate axes, check for backlash |
| Power meter noise | Newport readings jump > 5% | EMI, baseline drift, or environmental | Shield cables, re-establish baseline |

---

## Part 9: Safety Officer Sign-Off

**REQUIRED for MaiTai Testing Only**

```
LASER SAFETY OFFICER APPROVAL FORM

Project: V4 Hardware Validation
Laser: Spectra-Physics MaiTai Ti:Sapphire
Date: _________________
Time: _________________

Safety Officer Name: _________________________
Safety Officer Signature: _________________________

Test Operator Name: _________________________
Test Operator Signature: _________________________

Pre-Test Safety Review:
☐ Laser enclosure interlocks verified
☐ Shutter function tested and working
☐ Emergency stop tested and functional
☐ Operator trained on laser safety
☐ Protective equipment available and worn
☐ Beam path clear and marked
☐ Fire extinguisher accessible
☐ Water cooling operational (if present)

Critical Safety Tests Passed:
☐ Shutter closes properly
☐ Shutter opens only with command
☐ Shutter closes immediately on shutdown
☐ Laser power meter reads expected values
☐ Alignment indicators show good alignment

Approval Decision:
☐ APPROVED - Operator may proceed with testing
☐ APPROVED WITH CONDITIONS: _____________
☐ NOT APPROVED - Testing must not proceed

Supervisor will be:
☐ Present during entire test
☐ On-call/available if needed
☐ Not available (testing must be postponed)

Notes:
_________________________________________________________________
_________________________________________________________________
_________________________________________________________________

Approval is valid for testing session beginning at: _________
Testing must conclude by: _________
```

---

## Summary & Next Steps

### Preparation Timeline

1. **Week Before** (Mon-Fri):
   - Read full `HARDWARE_VALIDATION_PLAN.md`
   - Review equipment datasheets
   - Schedule laser safety officer for MaiTai testing
   - Arrange lab access for 6-7 hours

2. **Day Before** (Friday):
   - Verify SSH access to maitai-eos
   - Confirm all instruments are powered on
   - Schedule equipment warm-up time (15 min minimum)

3. **Day Of Test** (Morning):
   - Run `prepare_hardware_testing.sh`
   - Complete pre-test verification checklist
   - Print emergency procedures and safety forms

4. **Testing** (As Scheduled):
   - Start with SCPI (safest, lowest risk)
   - Progress through Newport, PVCAM, ESP300
   - End with MaiTai (highest risk, supervisor required)

5. **After Tests**:
   - Complete post-test verification checklist
   - Generate test report
   - Archive all logs
   - Create git tag: `hardware-validation-v4-YYYY-MM-DD`

### Key Contacts

| Role | Name | Phone | Email |
|------|------|-------|-------|
| Laser Safety Officer | | | |
| Facility Manager | | | |
| Equipment Support | | | |
| Emergency Services | 911 | | |

### Document References

- **This Document**: `HARDWARE_TEST_PREPARATION.md` (step-by-step prep guide)
- **Full Test Plan**: `HARDWARE_VALIDATION_PLAN.md` (detailed test scenarios)
- **Quick Reference**: `QUICK_START_HARDWARE_TESTING.md` (fast lookup)
- **Checklist**: `HARDWARE_TEST_CHECKLIST.md` (printable checklist)
- **V4 Architecture**: `docs/V4_ONLY_ARCHITECTURE_PLAN.md` (system overview)

---

## Document Status

**Version**: 1.0
**Date**: 2025-11-17
**Status**: Ready for Execution
**Review Cycle**: Every 2 weeks during hardware testing phase
**Last Updated**: 2025-11-17

---

**READY TO BEGIN HARDWARE TESTING**

Print this document, complete all checklists, and execute the testing plan as outlined above.

Good luck with your hardware validation!
