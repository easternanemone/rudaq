# V4 Hardware Testing - Quick Start Guide

**Time to First Test**: 10 minutes
**Total Testing Time**: 6-7 hours over 2-3 days

---

## 30-Second Overview

Five V4 actors need real hardware validation after successful mock testing:
1. **SCPI** (generic SCPI instruments)
2. **Newport 1830-C** (optical power meter)
3. **PVCAM** (camera sensor)
4. **ESP300** (motion controller)
5. **MaiTai** (tunable laser - requires safety officer)

All hardware is on `maitai-eos` cluster accessible via SSH.

---

## Quick Setup (5 minutes)

```bash
# 1. SSH to maitai-eos
ssh maitai@maitai-eos

# 2. Navigate to V4 project
cd ~/rust-daq/v4-daq

# 3. Enable debug logging
export RUST_LOG=debug

# 4. Build all examples
cargo build --examples --release

# 5. Create results directory
mkdir -p ~/hardware_test_results/$(date +%Y-%m-%d)
```

---

## Testing Order (Safest to Riskiest)

### Day 1: SCPI + Newport (2 hours)
```bash
# SCPI Test (safest, lowest risk)
export SCPI_RESOURCE="GPIB0::10::INSTR"  # Verify with visainfo first
cargo run --example v4_scpi_hardware_test --release | tee scpi_$(date +%s).log
# Expected: All messages show ✓

# Newport 1830-C Test (low risk, measurement instrument)
export NEWPORT_PORT="/dev/ttyUSB0"  # Find actual port with: ls /dev/ttyUSB*
cargo run --example v4_newport_hardware_test --release | tee newport_$(date +%s).log
# Expected: Power readings stable < 2% variation
```

### Day 2: PVCAM + ESP300 (3 hours)
```bash
# PVCAM Test (medium risk, optical/camera)
cargo run --example v4_pvcam_hardware_test --release | tee pvcam_$(date +%s).log
# Expected: Frames captured at ~9 fps

# ESP300 Test (medium risk, motion - requires caution)
export ESP300_PORT="/dev/ttyUSB0"
cargo run --example v4_esp300_hardware_test --release | tee esp300_$(date +%s).log
# Expected: All axes home successfully, move with accuracy
```

### Day 3: MaiTai (1.5 hours, REQUIRES SUPERVISOR)
```bash
# CRITICAL: Laser safety officer must supervise
# Do NOT proceed without supervisor approval

export MAITAI_PORT="/dev/ttyUSB5"
cargo run --example v4_maitai_hardware_test --release | tee maitai_$(date +%s).log
# Expected: Shutter starts closed, wavelength tuning works, shutter closes on shutdown
```

---

## Pre-Test Verification (2 minutes)

```bash
# Verify all instruments present
visainfo          # Shows SCPI instruments
ls -la /dev/ttyUSB*  # Shows serial devices
lsusb | grep -i photometric  # Shows camera

# Verify warm-up complete (15 minutes after power on)
# Check all instrument displays show stable values
```

---

## Understanding Test Output

### Good Signs (Test Passing)
```
✓ Actor spawned
✓ Message sent successfully
✓ Response received: [value]
✓ Test completed
Graceful shutdown
```

### Bad Signs (Test Failing)
```
ERROR: Timeout
ERROR: Connection refused
thread 'main' panicked
No response from hardware
Unexpected shutdown
```

---

## Emergency Commands

```bash
# If test hangs, open another terminal:
pkill -f "cargo run --example"  # Kill stuck test

# If equipment becomes unsafe:
# For MaiTai: Hit emergency stop (facility specific)
# For ESP300: Unplug power or hit emergency stop
# For Camera: Disconnect USB

# Return all equipment to safe state:
# MaiTai: Verify shutter closed (SHUTTER?\r)
# ESP300: Return to home (HOME\r\n)
```

---

## Pass/Fail Quick Criteria

### SCPI: PASS if
- No ERROR messages
- All queries get responses
- Duration < 60 seconds
- Graceful shutdown

### Newport: PASS if
- Power readings stable (< 2% variation)
- Baseline < 100 uW
- No errors in output

### PVCAM: PASS if
- Frames captured (> 10)
- Frame rate ~9 fps
- No memory allocation errors

### ESP300: PASS if
- All axes home successfully
- Positions accurate within 0.2mm
- Returns to home at end
- Graceful shutdown

### MaiTai: PASS if (CRITICAL SAFETY)
- Shutter STARTS CLOSED
- Shutter ENDS CLOSED
- Wavelength tuning works
- No errors in logs

---

## Results Summary

After all tests complete:

```bash
# Archive results
cp *.log ~/hardware_test_results/$(date +%Y-%m-%d)/

# Generate summary
for log in *.log; do
  echo "=== $log ===" >> summary.txt
  grep -E "✓|ERROR|panic" "$log" >> summary.txt
done
cat summary.txt

# Verify mock tests still pass (regression check)
cargo test --test v4_scpi_integration_test --release
```

---

## Troubleshooting Reference

| Error | Solution |
|-------|----------|
| `Permission denied /dev/ttyUSB0` | `sudo usermod -a -G dialout $USER` and log out/in |
| `Connection refused` | Power cycle instrument and wait 15 seconds |
| `Timeout after 2 seconds` | Increase timeout in actor code to 5 seconds |
| `No VISA resources found` | Check cables, run `visainfo` to verify connection |
| `Camera not found` | Check USB 3.0 port, reload driver |
| Test hangs | Open new terminal and run: `pkill -f "cargo run"` |

---

## Contact Information

- **Hardware Issues**: Contact facility manager
- **Safety Emergency**: Call emergency services
- **Laser Safety**: Contact laser safety officer
- **Software Issues**: Review error logs, check git history for recent changes

---

## Document References

- **Full Plan**: `HARDWARE_VALIDATION_PLAN.md` (comprehensive, 50+ test scenarios)
- **Checklist**: `HARDWARE_TEST_CHECKLIST.md` (printable checklist for each test)
- **This Guide**: `QUICK_START_HARDWARE_TESTING.md` (quick reference)

---

**Version**: 1.0 | **Date**: 2025-11-17 | **Status**: Ready for Phase 2

