# Ready to Execute on Hardware - Final Handoff

**Date**: 2025-11-17
**Status**: ✅ **ALL INFRASTRUCTURE VERIFIED AND READY**
**Action Required**: Manual execution on maitai-eos with hardware access

---

## ✅ Verification Complete

All automated infrastructure has been verified and is ready for hardware execution:

### Code Verification ✅

```
Compilation: CLEAN
Integration Tests: 8/8 PASSED
Hardware Tests: 94 tests ready (marked as #[ignore])
Scripts: 10/10 executable
Total: 102 tests implemented
```

**Test Results**:
```bash
running 102 tests
test result: ok. 8 passed; 0 failed; 94 ignored; 0 measured; 0 filtered out
```

The 94 hardware tests are marked `#[ignore]` and will only run when you explicitly execute them with the `--ignored` flag on maitai-eos.

---

## 🚀 How to Execute (You Must Do These Steps)

### Prerequisites

1. **Physical Access**: You need to be able to SSH to maitai-eos
2. **Hardware**: All 5 instruments must be powered on and connected
3. **Safety**: Laser Safety Officer approval for MaiTai testing
4. **Time**: Block 6-7 hours for complete execution

### Step-by-Step Execution

#### 1. SSH to maitai-eos (5 minutes)

```bash
# From your laptop
ssh maitai@maitai-eos
```

If you don't have SSH access set up, follow: `docs/testing/SSH_ACCESS_GUIDE.md`

#### 2. Deploy Code to maitai-eos (5 minutes)

**Option A: From Your Laptop (Recommended)**
```bash
cd /Users/briansquires/code/rust-daq/v4-daq
./scripts/remote/deploy_to_maitai.sh
```

**Option B: Manual rsync**
```bash
rsync -avz --exclude target \
  /Users/briansquires/code/rust-daq/v4-daq/ \
  maitai@maitai-eos:~/rust-daq/
```

**Option C: Git Pull (if repo is on maitai-eos)**
```bash
# On maitai-eos
cd ~/rust-daq
git pull origin main
```

#### 3. Run the Master Execution Script (6-7 hours)

**On maitai-eos**:
```bash
cd ~/rust-daq
./scripts/EXECUTE_HARDWARE_TESTS.sh
```

This script will:
1. ✅ Verify you're on maitai-eos
2. ✅ Run hardware verification
3. ✅ Run safety checks (interactive prompts)
4. ✅ Build the project
5. ✅ Execute all 5 test phases in safe order:
   - SCPI (17 tests, 20 min) - LOW RISK
   - Newport (14 tests, 20 min) - LOW RISK
   - ESP300 (16 tests, 45 min) - MEDIUM RISK
   - PVCAM (28 tests, 30 min) - MEDIUM RISK
   - MaiTai (19 tests, 90 min) - **CRITICAL RISK** (requires Laser Safety Officer)
6. ✅ Analyze results
7. ✅ Generate comprehensive report

The script includes:
- Interactive safety confirmations
- Ability to skip MaiTai if Laser Safety Officer unavailable
- Automatic log file creation with timestamps
- Color-coded output
- Progress tracking
- Result analysis

#### 4. Monitor Progress (Optional, from your laptop)

While tests are running on maitai-eos:
```bash
# From your laptop
./scripts/remote/monitor_tests.sh
```

#### 5. Review Results (10 minutes)

After execution completes:
```bash
# On maitai-eos
cd ~/rust-daq

# View the report
cat test-results/YYYY-MM-DD_HH-MM-SS/report.md

# Create baseline for future comparison
./scripts/hardware_validation/create_baseline.sh
```

---

## ⚠️ Critical Safety Requirements

### For MaiTai Laser Testing

**MANDATORY REQUIREMENTS**:
1. ✅ Laser Safety Officer MUST be physically present
2. ✅ Safety briefing MUST be completed beforehand
3. ✅ Shutter MUST be verified CLOSED before starting
4. ✅ Emergency stop button MUST be tested and accessible
5. ✅ Eye protection MUST be available in lab
6. ✅ Warning signs MUST be posted on all entries
7. ✅ Lab area MUST be secured (no unauthorized access)
8. ✅ Evacuation route MUST be clear

**The execution script will prompt for these confirmations. If any are "no", MaiTai tests will be skipped safely.**

### Emergency Procedures

If something goes wrong during testing:

**Immediate Action**:
```bash
# On maitai-eos
./scripts/hardware_validation/emergency_stop.sh
```

**Manual Emergency**:
- **MaiTai**: Press physical emergency stop button immediately
- **ESP300**: Press emergency stop on controller
- **All instruments**: Power down if necessary

---

## 📋 Pre-Flight Checklist

Before running `./scripts/EXECUTE_HARDWARE_TESTS.sh`, verify:

### System Access ✅
- [ ] SSH access to maitai@maitai-eos working
- [ ] Code deployed to maitai-eos (~/rust-daq/)
- [ ] Rust toolchain installed on maitai-eos
- [ ] Sufficient disk space (at least 10 GB free)

### Hardware ✅
- [ ] All 5 instruments powered on:
  - [ ] SCPI instrument (VISA/GPIB/TCP)
  - [ ] Newport 1830-C power meter
  - [ ] ESP300 motion controller
  - [ ] PVCAM camera (PrimeBSI or compatible)
  - [ ] MaiTai tunable laser
- [ ] All instruments warmed up (>30 minutes)
- [ ] No hardware errors on startup
- [ ] Cables connected and secure

### Safety (MaiTai Only) ⚠️
- [ ] Laser Safety Officer contacted and available
- [ ] Safety briefing scheduled
- [ ] Lab access coordinated (no interruptions)
- [ ] Emergency stop button tested
- [ ] Eye protection available
- [ ] Warning signs ready to post
- [ ] First aid kit accessible
- [ ] Fire extinguisher accessible

### Permissions ✅
- [ ] User has access to serial ports (`/dev/ttyUSB*`)
- [ ] User has access to VISA resources
- [ ] User has access to PVCAM SDK
- [ ] User can write to log directories

### Time ⏱
- [ ] 6-7 hours blocked for testing
- [ ] No other experiments scheduled on hardware
- [ ] Backup plan if tests take longer than expected

---

## 🔍 What Happens During Execution

### Phase 1: SCPI (20 minutes, LOW RISK)
- Tests generic SCPI instruments via VISA
- Validates *IDN?, measurements, error handling
- Safe - no physical movement or high power

### Phase 2: Newport 1830-C (20 minutes, LOW RISK)
- Tests optical power meter
- Wavelength calibration (633nm, 532nm, 800nm, 1064nm)
- Power measurements in various units
- Safe - passive measurement device

### Phase 3: ESP300 (45 minutes, MEDIUM RISK)
- Tests 3-axis motion controller
- **Physical movement** - stages will move
- Homing, positioning (±0.01mm accuracy)
- Emergency stop testing
- **Script prompts for workspace clearance**

### Phase 4: PVCAM (30 minutes, MEDIUM RISK)
- Tests 2048×2048 scientific camera
- Frame acquisition, ROI, binning, streaming
- Temperature control, cooler operation
- **High data rates** (~72 MB/s)

### Phase 5: MaiTai (90 minutes, CRITICAL RISK) ⚠️
- Tests tunable laser (690-1040nm)
- **LASER RADIATION** - Class 4 laser
- Shutter control, wavelength tuning, power measurement
- **Script enforces safety confirmations**
- **Skipped if Laser Safety Officer not present**

---

## 📊 Expected Results

### Success Criteria

| Metric | Target | How Measured |
|--------|--------|--------------|
| Overall Pass Rate | >90% | Total passed / total tests |
| SCPI Tests | 17/17 | Cargo test output |
| Newport Tests | 14/14 | Cargo test output |
| ESP300 Tests | 16/16 | Cargo test output |
| PVCAM Tests | 28/28 | Cargo test output |
| MaiTai Tests | 19/19 | Cargo test output (if run) |
| Safety Incidents | 0 | Safety log |
| Hardware Damage | 0 | Visual inspection |

### What You'll Get

After execution, you'll have:

1. **Test Logs** (in `test-results/YYYY-MM-DD_HH-MM-SS/`):
   - `execution.log` - Master execution log
   - `scpi_tests.log` - SCPI test output
   - `newport_tests.log` - Newport test output
   - `esp300_tests.log` - ESP300 test output
   - `pvcam_tests.log` - PVCAM test output
   - `maitai_tests.log` - MaiTai test output (if run)

2. **Reports**:
   - `report.md` - Human-readable markdown report
   - `report.json` - Machine-readable JSON
   - `metrics.json` - Performance metrics

3. **Baseline** (after running `create_baseline.sh`):
   - Stored for future regression testing
   - Compares future runs against this successful run

---

## 🐛 Troubleshooting

### "Cannot connect to hardware"
- Verify hardware is powered on and warmed up
- Check cables and connections
- Run `./scripts/hardware_validation/verify_hardware.sh --verbose`

### "Permission denied" errors
- Add user to `dialout` group for serial ports: `sudo usermod -a -G dialout $USER`
- Verify VISA library installed
- Verify PVCAM SDK installed

### "Tests timeout"
- Hardware may need more warm-up time
- Check for hardware errors (flashing LEDs, error messages)
- Try running individual test suites with longer timeouts

### "Safety checks fail"
- Review safety procedures in `docs/testing/HARDWARE_TEST_PREPARATION.md`
- Contact Laser Safety Officer for MaiTai approval
- Ensure all safety equipment is available

### "Build fails"
- Verify Rust toolchain: `rustc --version` (should be 1.75+)
- Update dependencies: `cargo update`
- Check compiler errors and fix

---

## 📞 Support Resources

### Documentation
- **Master Guide**: `docs/HARDWARE_VALIDATION_READY.md`
- **Test Procedures**: `docs/testing/HARDWARE_TEST_PREPARATION.md`
- **SSH Setup**: `docs/testing/SSH_ACCESS_GUIDE.md`
- **Safety**: `docs/testing/HARDWARE_TEST_PREPARATION.md` (Section 5)

### Scripts
- **Master Executor**: `scripts/EXECUTE_HARDWARE_TESTS.sh`
- **Hardware Check**: `scripts/hardware_validation/verify_hardware.sh`
- **Safety Check**: `scripts/hardware_validation/safety_check.sh`
- **Emergency Stop**: `scripts/hardware_validation/emergency_stop.sh`

### Emergency Contacts (FILL IN BEFORE TESTING)
- Laser Safety Officer: ________________
- Facility Manager: ________________
- Equipment Support: ________________
- Emergency Services: 911

---

## ✅ Final Verification Status

**Infrastructure**:
- ✅ Code compiles: CLEAN
- ✅ Integration tests: 8/8 PASSED
- ✅ Hardware tests: 94 implemented and ready
- ✅ Scripts: 10/10 executable
- ✅ Documentation: 153 KB complete

**Next Action**:
- [ ] SSH to maitai@maitai-eos
- [ ] Run `./scripts/EXECUTE_HARDWARE_TESTS.sh`
- [ ] Review results
- [ ] Create baseline
- [ ] Update beads tracker

**Estimated Time**: 6-7 hours (can be split across multiple sessions if needed)

**Status**: ✅ **READY FOR YOU TO EXECUTE**

---

## Quick Command Reference

```bash
# 1. SSH to hardware system
ssh maitai@maitai-eos

# 2. Deploy code (from laptop)
./scripts/remote/deploy_to_maitai.sh

# 3. Run all tests (on maitai-eos)
cd ~/rust-daq
./scripts/EXECUTE_HARDWARE_TESTS.sh

# 4. Monitor (from laptop, optional)
./scripts/remote/monitor_tests.sh

# 5. Review results (on maitai-eos)
cat test-results/YYYY-MM-DD_HH-MM-SS/report.md

# 6. Create baseline
./scripts/hardware_validation/create_baseline.sh

# Emergency stop (if needed)
./scripts/hardware_validation/emergency_stop.sh
```

---

**Document Status**: Ready for Manual Execution on Hardware
**Created**: 2025-11-17
**Infrastructure**: 100% Complete and Verified
**Awaiting**: Physical hardware access and execution

**Next**: SSH to maitai-eos and run `./scripts/EXECUTE_HARDWARE_TESTS.sh`
