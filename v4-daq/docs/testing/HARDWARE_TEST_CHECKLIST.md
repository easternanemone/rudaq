# Hardware Test Execution Checklist

**Quick Reference for Hardware Validation Testing**
**Use alongside**: `HARDWARE_VALIDATION_PLAN.md`

---

## Pre-Test Preparation (30 minutes before first test)

### Environment Setup
- [ ] Lab temperature stable (within 2°C)
- [ ] No excessive vibration from HVAC/machinery
- [ ] Adequate ventilation confirmed
- [ ] Ambient lighting adequate (especially for camera)
- [ ] Safety equipment accessible (goggles, first aid kit)
- [ ] Emergency contact list posted

### Hardware Warm-Up
- [ ] All instruments powered on
- [ ] 15-minute warm-up completed for all hardware
- [ ] All serial cables physically connected and tested
- [ ] Network connections verified (for SCPI over Ethernet)
- [ ] USB devices recognized by system (`lsusb`)
- [ ] All serial ports accessible (`ls /dev/ttyUSB*`)

### Mechanical Verification
- [ ] ESP300 axes at home position
- [ ] MaiTai laser shutter closed (safety check)
- [ ] Newport 1830-C baseline zeroed
- [ ] PVCAM camera lens focused
- [ ] No physical obstructions in beam paths
- [ ] All cables not pinched or damaged

### Software Preparation
- [ ] V4 project cloned and updated (`git pull`)
- [ ] All dependencies installed (`cargo tree | head -20`)
- [ ] Project compiles successfully (`cargo build --release`)
- [ ] All examples built (`cargo build --examples --release`)
- [ ] Logging configured (`export RUST_LOG=debug`)
- [ ] Test result directory created (`mkdir -p ~/hardware_test_results/$(date +%Y-%m-%d)`)
- [ ] Git repository clean (`git status`)

### Documentation
- [ ] Hardware serial numbers recorded
- [ ] Calibration certificates available locally
- [ ] Datasheets saved and accessible
- [ ] Test result template prepared
- [ ] Emergency procedures posted
- [ ] Contact information available

---

## SSH Connection & Environment

### Connect to maitai-eos
```bash
ssh maitai@maitai-eos
# Alternative: ssh maitai@100.91.139.90
```

**Verification After Connection**:
- [ ] Prompt shows: `maitai@maitai-eos:~$`
- [ ] Can list hardware: `ls /dev/ttyUSB*` (shows devices)
- [ ] Can verify VISA: `visainfo` (lists instruments) OR `pkg-config --list-all | grep visa`

### Test Directory Setup
```bash
cd ~/rust-daq/v4-daq
pwd  # Verify correct directory
```

- [ ] Correct directory: `/Users/briansquires/code/rust-daq/v4-daq` (or equivalent)
- [ ] Git status clean (no uncommitted changes from setup)
- [ ] Can see examples: `ls examples/v4_*_hardware_test.rs`

---

## SCPI Actor Testing (Start Here - Safest)

### Pre-Test (5 minutes)
```bash
# Identify SCPI instruments
visainfo  # Note the RESOURCE string for your instrument
```

- [ ] SCPI instrument visible in visainfo output
- [ ] RESOURCE string captured (e.g., `GPIB0::10::INSTR`)

### Test Execution (10 minutes)
```bash
# Set environment variable with your RESOURCE string
export SCPI_RESOURCE="GPIB0::10::INSTR"  # Example, use your actual string

# Run hardware test
cargo run --example v4_scpi_hardware_test --release 2>&1 | tee scpi_test_$(date +%Y%m%d_%H%M%S).log
```

**Monitor Output For**:
- [ ] `✓ Actor spawned` message appears
- [ ] `✓ *IDN? Response` contains instrument name
- [ ] `✓ *STB? Query` returns numeric value
- [ ] `✓ Query test` completes
- [ ] `✓ Timeout handling` works
- [ ] `✓ Graceful shutdown` message
- [ ] Total duration: < 60 seconds

### Test Validation (5 minutes)
```bash
# Check for errors
grep -E "ERROR|panic|thread.*panicked" scpi_test_*.log

# If no output from grep, test passed
if [ $? -ne 0 ]; then
  echo "✅ SCPI Test PASSED - No errors found"
else
  echo "❌ SCPI Test FAILED - See errors above"
  # Show last 30 lines for diagnosis
  tail -30 scpi_test_*.log
fi
```

**Pass Criteria**:
- [ ] No ERROR messages in logs
- [ ] No panic/crash messages
- [ ] All queries received responses
- [ ] Duration < 60 seconds
- [ ] Graceful shutdown confirmed

**Result**: [ ] PASS [ ] FAIL

---

## Newport 1830-C Power Meter Testing

### Pre-Test (10 minutes)

#### Manual Verification
```bash
# 1. Identify serial port
ls -la /dev/ttyUSB*  # Find Newport port (usually lowest number)

# 2. Test connection
picocom -b 9600 /dev/ttyUSB0  # Replace 0 with correct number

# In picocom:
# Type: PM:ZeroBaseline
# Press Enter
# Expected: OK or blank line (no error)

# Type: PM:Lambda 800
# Press Enter
# Expected: OK or blank line

# Type: PM:ReadPower?
# Press Enter
# Expected: 0.000000 or small value (dark baseline)

# Exit picocom: Ctrl-A, Ctrl-X
```

- [ ] Serial port found and accessible
- [ ] Manual zero command succeeded
- [ ] Wavelength set to 800 nm
- [ ] Baseline power < 100 uW (5 zeros after decimal)

### Test Execution (5 minutes)
```bash
# Set environment variable
export NEWPORT_PORT="/dev/ttyUSB0"  # Use your actual port

# Run test
cargo run --example v4_newport_hardware_test --release 2>&1 | tee newport_test_$(date +%Y%m%d_%H%M%S).log
```

**Monitor Output For**:
- [ ] Actor spawns successfully
- [ ] Baseline power displayed
- [ ] Power measurements stable
- [ ] Wavelength setting confirmed
- [ ] Units display correctly (Watts or dBm)

### Test Validation (5 minutes)
```bash
# Check results
grep -E "Power:|Error|panic" newport_test_*.log

# Verify stability (last 5 power readings should be within 2%)
tail -20 newport_test_*.log
```

**Pass Criteria**:
- [ ] No error messages
- [ ] Power readings stable (< 2% variation)
- [ ] Baseline established < 100 uW
- [ ] Graceful shutdown

**Result**: [ ] PASS [ ] FAIL

---

## PVCAM Actor Testing

### Pre-Test (10 minutes)

#### Camera Verification
```bash
# Check camera is recognized
lsusb | grep -i photometrics

# Expected output shows camera device (e.g., "Teledyne Photometrics PRIME")

# Check PVCAM library
pkg-config --list-all | grep pvc

# Expected: Shows pvcam library version
```

- [ ] Camera visible in lsusb
- [ ] PVCAM library installed
- [ ] Camera lens focused on test target
- [ ] Adequate lighting for camera

### Test Execution (15 minutes)
```bash
# Run PVCAM test
cargo run --example v4_pvcam_hardware_test --release 2>&1 | tee pvcam_test_$(date +%Y%m%d_%H%M%S).log
```

**Monitor During Test**:
- [ ] Sensor initialized: 2048x2048 resolution confirmed
- [ ] Capabilities queried successfully
- [ ] Frame data received (shows frame numbers)
- [ ] Streaming starts and stops cleanly
- [ ] ROI changes applied correctly
- [ ] No memory warnings in logs

### Test Validation (5 minutes)
```bash
# Check frame counts
grep "Frame #" pvcam_test_*.log | wc -l  # Should show number of frames

# Check for memory issues
grep -i "alloc\|memory\|leak" pvcam_test_*.log

# If no output, memory test passed
if [ $? -ne 0 ]; then
  echo "✅ Memory check passed"
else
  echo "⚠️ Review memory messages above"
fi
```

**Pass Criteria**:
- [ ] Camera initializes without error
- [ ] Frames captured successfully (> 10 frames)
- [ ] Frame rate approximately 9 fps (9-11 fps acceptable)
- [ ] No memory allocation errors
- [ ] Streaming stops cleanly

**Result**: [ ] PASS [ ] FAIL

---

## ESP300 Motion Controller Testing (REQUIRES CAUTION)

### Safety Briefing (5 minutes) - MANDATORY

**Read Before Proceeding**:
- [ ] I understand ESP300 can cause pinch injuries
- [ ] I will keep hands clear during motion
- [ ] I will use maximum 10 mm/s speed during validation
- [ ] I know where emergency stop is located
- [ ] I have verified soft limits are configured

### Pre-Test (10 minutes)

#### Manual Verification
```bash
# 1. Identify serial port
ls -la /dev/ttyUSB*

# 2. Test connection
picocom -b 19200 /dev/ttyUSB0  # Use correct port

# In picocom:
# Type: HOME
# Press Enter twice (\r\n)
# Wait for axes to move home (should hear/see motion)
# Expected response: "0,0,0" or similar

# Type: TP:1
# Press Enter twice
# Expected: Position of axis 1 (should be 0.000 after home)

# Exit: Ctrl-A, Ctrl-X
```

- [ ] Serial connection established
- [ ] Axes respond to HOME command
- [ ] All axes reach home position (0,0,0)
- [ ] Position query works

#### Configure Soft Limits (CRITICAL)
```bash
# In picocom, set conservative soft limits:
# Type: LIMIT:ABS 0,-50,50
# Press Enter twice
# Expected: No error

# Type: LIMIT:ABS 1,-50,50
# Press Enter twice

# Type: LIMIT:ABS 2,-50,50
# Press Enter twice

# Verify limits set:
# Type: LIMIT:ABS? 0
# Press Enter twice
# Expected: -50,50
```

- [ ] Soft limits configured for all axes
- [ ] Limits set to -50 to +50 mm (conservative)
- [ ] Limits verified via query

### Test Execution (15 minutes)

```bash
# Set environment
export ESP300_PORT="/dev/ttyUSB0"
export ESP300_AXES="3"  # Number of axes (usually 3)

# Run motion test
cargo run --example v4_esp300_hardware_test --release 2>&1 | tee esp300_test_$(date +%Y%m%d_%H%M%S).log

# OBSERVE STAGE DURING TEST:
# - Motion should be smooth, not jerky
# - Axes should return to home after test
# - No grinding or alarm sounds
```

**During Test - Watch For**:
- [ ] Axes move smoothly (no grinding)
- [ ] Motion direction correct
- [ ] Axes return to home position
- [ ] All motion completes successfully
- [ ] No alarm or error sounds

### Test Validation (5 minutes)

```bash
# Check test results
grep -E "Position|ERROR|panic" esp300_test_*.log

# Verify homing
grep "Home" esp300_test_*.log | tail -5

# Check final position
tail -10 esp300_test_*.log | grep -E "Position|0\.0"
```

**Pass Criteria**:
- [ ] All axes home successfully
- [ ] Moves reach target position (within 0.2mm)
- [ ] Final position returns to home (0.0mm)
- [ ] No motion errors or stalls
- [ ] Graceful shutdown

**Post-Test Safety Check**:
```bash
# Verify axes at home
picocom -b 19200 /dev/ttyUSB0
# In picocom: TP:1
# Expected: 0.000
# Type: TP:2, TP:3 - verify all are 0.000
# Exit: Ctrl-A, Ctrl-X
```

- [ ] All axes confirmed at 0.0 position after test

**Result**: [ ] PASS [ ] FAIL

---

## MaiTai Laser Testing (REQUIRES LASER SAFETY OFFICER)

### CRITICAL SAFETY BRIEFING (20 minutes) - MANDATORY

**Supervisor Approval Required**:
- [ ] Laser Safety Officer present: _________________ (Print Name)
- [ ] Laser Safety Officer signature: _________________ (Signature)
- [ ] Date/Time: _________________________________

**I confirm I have read and understand**:
- [ ] MaiTai is Class 4 laser (can cause eye damage)
- [ ] All personnel wear appropriate safety glasses
- [ ] Shutter must be closed when not testing
- [ ] Beam path is enclosed and controlled
- [ ] Emergency stop procedure reviewed
- [ ] Medical attention contact available

### Pre-Test Laser Checks (15 minutes)

#### Physical Inspection
- [ ] Laser enclosure intact, no cracks
- [ ] All covers in place, interlocks functional
- [ ] Safety shutter operational (manual test)
- [ ] Emergency stop button accessible and labeled
- [ ] Beam path clear and unobstructed
- [ ] Water cooling connected (if applicable)
- [ ] Thermal management status OK
- [ ] Power supply connected and ready
- [ ] All personnel have safety glasses on

#### Manual Verification
```bash
# 1. Identify serial port
ls -la /dev/ttyUSB*  # Usually USB5 for MaiTai

# 2. Test connection
picocom -b 9600 /dev/ttyUSB5  # Use your actual port

# In picocom:
# Type: SHUTTER?
# Press Enter
# Expected: 0 (closed) - CRITICAL SAFETY CHECK

# Type: WAVELENGTH?
# Press Enter
# Expected: Number between 700-1000 (current wavelength)

# Type: SHUTTER:0
# Press Enter
# Expected: No error (confirms shutter close command)

# Exit: Ctrl-A, Ctrl-X
```

- [ ] Shutter is CLOSED (0) before testing
- [ ] Wavelength query successful
- [ ] Shutter command accepted

### Test Execution (15 minutes)

```bash
# Set environment
export MAITAI_PORT="/dev/ttyUSB5"  # Use correct port
export MAITAI_BAUD="9600"

# Run MaiTai test
cargo run --example v4_maitai_hardware_test --release 2>&1 | tee maitai_test_$(date +%Y%m%d_%H%M%S).log

# OBSERVE DURING TEST:
# - Power display should show changes as wavelength varies
# - Shutter open/close should be audible or visible
# - No sparks, smoke, or unusual sounds
```

**During Test - Monitor For**:
- [ ] Test starts cleanly (no immediate errors)
- [ ] Wavelength changes visible on laser display
- [ ] Power output changes as wavelength changes
- [ ] Shutter responds to open/close commands
- [ ] No strange noises or smells
- [ ] All test stages complete
- [ ] Test finishes gracefully

### Test Validation (5 minutes)

```bash
# Check final shutter state - CRITICAL
tail -5 maitai_test_*.log | grep -i "shutter"

# Verify test completed
grep -E "✓|PASS" maitai_test_*.log

# Check for errors
grep -E "ERROR|panic" maitai_test_*.log
```

**Pass Criteria**:
- [ ] Initial shutter state closed (safe default)
- [ ] Shutter open/close commands work
- [ ] Wavelength tuning works (700-1000nm)
- [ ] Power output measured
- [ ] **CRITICAL: Shutter closed at shutdown**
- [ ] No error messages
- [ ] Graceful test completion

### Post-Test Laser Safety (10 minutes)

```bash
# 1. Verify shutter is CLOSED
picocom -b 9600 /dev/ttyUSB5
# Type: SHUTTER?
# Expected: 0 (closed)
# Exit: Ctrl-A, Ctrl-X

# 2. Power down if needed (follow laser manufacturer shutdown)
# Wait 5 minutes for cool-down before next test
```

- [ ] Shutter confirmed CLOSED
- [ ] Laser powered down safely (if needed)
- [ ] Cool-down time waited

**Laser Safety Officer Sign-Off**:
- [ ] Test completed safely
- [ ] All laser safety procedures followed
- [ ] No incidents or hazards observed
- [ ] Officer Name: _________________ (Print)
- [ ] Officer Signature: _________________ (Signature)

**Result**: [ ] PASS [ ] FAIL

---

## Test Summary & Results

### Results Table

| Actor | Status | Duration | Notes |
|-------|--------|----------|-------|
| SCPI | [ ] PASS [ ] FAIL | ___ min | |
| Newport 1830-C | [ ] PASS [ ] FAIL | ___ min | |
| PVCAM | [ ] PASS [ ] FAIL | ___ min | |
| ESP300 | [ ] PASS [ ] FAIL | ___ min | |
| MaiTai | [ ] PASS [ ] FAIL | ___ min | |

**Total Test Time**: ______ hours ______ minutes

### Overall Result

- [ ] **ALL TESTS PASSED** - Ready for Phase 2
- [ ] **SOME TESTS FAILED** - Review failures below
- [ ] **CRITICAL FAILURES** - Do not proceed

### Failed Test Details

**Test Name**: _____________________
**Expected**: _____________________
**Actual**: _____________________
**Root Cause**: _____________________
**Resolution**: _____________________

---

## Post-Test Activities

### Results Archival
```bash
# Create backup of test results
mkdir -p ~/hardware_test_results/$(date +%Y-%m-%d)

# Copy all logs
cp *.log ~/hardware_test_results/$(date +%Y-%m-%d)/

# Create summary report
echo "Hardware Validation Test Summary" > summary.txt
echo "================================" >> summary.txt
echo "Date: $(date)" >> summary.txt
echo "Total Tests: [Count]" >> summary.txt
echo "Passed: [Count]" >> summary.txt
echo "Failed: [Count]" >> summary.txt
cp summary.txt ~/hardware_test_results/$(date +%Y-%m-%d)/
```

- [ ] All logs copied to backup location
- [ ] Summary report created
- [ ] Results directory documented

### Regression Testing
```bash
# Verify mock tests still pass (confirm no regression)
cargo test --test v4_scpi_integration_test --release
cargo test --test integration_actors_test --release

# Expected: All tests pass (20/20)
```

- [ ] Mock tests still passing (no regression)
- [ ] All integration tests pass

### Documentation Update
```bash
# Document results in main repository
git add docs/testing/HARDWARE_VALIDATION_PLAN.md
git commit -m "docs: hardware validation testing completed $(date +%Y-%m-%d)"
git push origin main
```

- [ ] Test results documented
- [ ] Changes committed to git
- [ ] Changes pushed to repository

### Sign-Off & Approval

**Test Operator**:
- Name: _________________ (Print)
- Signature: _________________ (Signature)
- Date: _________________

**Technical Lead**:
- Name: _________________ (Print)
- Signature: _________________ (Signature)
- Date: _________________

**Safety Officer** (for MaiTai test):
- Name: _________________ (Print)
- Signature: _________________ (Signature)
- Date: _________________

---

## Post-Test Troubleshooting

### If Tests Fail

**Step 1: Check Logs**
```bash
tail -50 [actor]_test_*.log
```

**Step 2: Review Diagnosis Section in HARDWARE_VALIDATION_PLAN.md**
- Section 4.4 has common issues and solutions

**Step 3: Manual Verification**
- Rerun manual tests from Pre-Test sections
- Verify hardware still operational
- Check serial connections

**Step 4: Escalation**
- If hardware appears broken: Contact equipment support
- If software issue: Review error logs, check for recent code changes
- If network issue: Check Tailscale/SSH connectivity

### Known Issues & Workarounds

| Issue | Symptom | Workaround |
|-------|---------|-----------|
| VISA timeout | Test hangs after query | Increase timeout in actor code to 5s |
| Serial port busy | "Device in use" error | Restart picocom session, verify no other processes using port |
| Camera not detected | PVCAM shows no cameras | Reconnect USB cable, check USB3 port, load driver |
| Position not at home | ESP300 shows non-zero position | Manually run HOME command in picocom, retry test |
| Shutter won't open | MaiTai test times out | Check laser power, verify shutter mechanism, power cycle laser |

---

## Quick Reference Commands

```bash
# All-in-one pre-test setup
cd ~/rust-daq/v4-daq
export RUST_LOG=debug
cargo build --examples --release
mkdir -p ~/hardware_test_results/$(date +%Y-%m-%d)

# Test one actor
export SCPI_RESOURCE="GPIB0::10::INSTR"
cargo run --example v4_scpi_hardware_test --release 2>&1 | tee test_$(date +%Y%m%d_%H%M%S).log

# Review results
tail -20 test_*.log
grep -E "✓|ERROR" test_*.log

# Archive results
mv *.log ~/hardware_test_results/$(date +%Y-%m-%d)/
```

---

**Document Version**: 1.0
**Last Updated**: 2025-11-17
**Status**: Ready for Phase 2 Hardware Testing

