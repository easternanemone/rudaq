# V4 Hardware Testing - Complete Documentation Index

**Status**: Ready for Execution
**Date**: 2025-11-17
**Scope**: 89 hardware test scenarios across 5 V4 actors (SCPI, ESP300, PVCAM, Newport 1830-C, MaiTai)
**Duration**: 6-7 hours over 2-3 days
**Location**: maitai-eos cluster (SSH access via Tailscale)

---

## Documentation Overview

This directory contains complete preparation, execution, and reference materials for V4 hardware validation testing.

### Quick Navigation

**Just starting?** → Read [HARDWARE_TESTING_SUMMARY.md](HARDWARE_TESTING_SUMMARY.md) first (5-minute overview)

**Need step-by-step instructions?** → Follow [HARDWARE_TEST_PREPARATION.md](HARDWARE_TEST_PREPARATION.md) (detailed procedures)

**Ready to execute tests?** → Use [HARDWARE_TEST_CHECKLIST.md](HARDWARE_TEST_CHECKLIST.md) (printable, hand-fillable)

**Need detailed test scenarios?** → See [HARDWARE_VALIDATION_PLAN.md](HARDWARE_VALIDATION_PLAN.md) (all 94 tests)

**Need quick reference?** → Use [QUICK_START_HARDWARE_TESTING.md](QUICK_START_HARDWARE_TESTING.md) (10-minute lookup)

**Want automation scripts?** → Execute [HARDWARE_TESTING_SCRIPTS.sh](HARDWARE_TESTING_SCRIPTS.sh) on maitai-eos

---

## Document Details

### 1. HARDWARE_TESTING_SUMMARY.md (Overview)
**Size**: ~8 KB | **Read Time**: 5 minutes | **When to Read**: First

Quick overview of everything that's been prepared:
- What documents exist and their purposes
- Critical information summary
- Getting started quick guide
- Test schedule and distribution
- Success criteria

**Best For**: Initial understanding, quick reference

---

### 2. HARDWARE_TEST_PREPARATION.md (Detailed Guide)
**Size**: ~42 KB | **Read Time**: 30 minutes | **When to Read**: Before testing

Comprehensive step-by-step preparation guide:
- Part 1: SSH access verification (5 min)
- Part 2: Hardware availability checklist (10 min)
- Part 3: Safety procedures (critical)
  - General safety briefing
  - MaiTai laser safety (MANDATORY)
  - Motion control safety (ESP300)
  - Electrical safety checklist
- Part 4: Test execution schedule
- Part 5: Environment setup scripts
- Part 6: Success verification checklist
- Part 7: Emergency procedures
- Part 8: Troubleshooting reference
- Part 9: Laser safety officer sign-off form

**Best For**: Complete procedures, safety protocols, detailed setup

---

### 3. HARDWARE_TEST_CHECKLIST.md (Printable Checklist)
**Size**: ~18 KB | **Format**: Hand-fillable | **When to Use**: During testing

Day-by-day execution checklist organized as:
- **Day 1**: SCPI + Newport 1830-C (2 hours, low risk)
- **Day 2**: PVCAM + ESP300 (3 hours, medium risk)
- **Day 3**: MaiTai Laser (1.5 hours, critical risk, requires supervisor)
- Post-testing: Results archiving and regression testing

Each section includes:
- Pre-test checklist
- Test execution steps
- Monitoring during test
- Physical observations
- Results verification
- Sign-off

**Best For**: Tracking progress during execution, ensuring nothing is missed

---

### 4. HARDWARE_VALIDATION_PLAN.md (Detailed Test Plan)
**Size**: ~51 KB | **Read Time**: 45 minutes | **When to Read**: For reference

Comprehensive test scenarios for all 94 tests:
- Section 1: Hardware access configuration
- Section 2: Hardware requirements by actor
- Section 3: Test scenarios (detailed descriptions of all tests)
  - SCPI: 17 tests
  - ESP300: 16 tests
  - MaiTai: 19 tests
  - Newport 1830-C: 14 tests
  - PVCAM: 28 tests
- Section 4: Test execution guide with step-by-step examples
- Section 5: Safety considerations (detailed)
- Section 6: Timeline & scheduling
- Section 7: Success criteria & pass/fail matrix
- Section 8: Post-test analysis & reporting
- Appendices: Reference materials

**Best For**: Understanding test details, troubleshooting specific test issues

---

### 5. QUICK_START_HARDWARE_TESTING.md (Quick Reference)
**Size**: ~5.4 KB | **Read Time**: 10 minutes | **When to Use**: Quick lookup

Fast reference for executing tests:
- 30-second overview
- Quick setup (5 minutes)
- Testing order (safest to riskiest)
- Pre-test verification (2 minutes)
- Understanding test output
- Emergency commands
- Pass/fail quick criteria
- Results summary

**Best For**: Quick lookups, emergency reference during testing

---

### 6. HARDWARE_TESTING_SCRIPTS.sh (Automation Scripts)
**Size**: ~4 KB | **Format**: Bash script | **When to Use**: On maitai-eos

Four automated scripts for testing:

**Script 1: prepare_hardware_testing.sh**
- Creates results directory
- Sets environment variables
- Builds all examples
- Records baseline measurements
- Creates test logging utilities
- Time: 10 minutes

**Script 2: run_hardware_tests.sh**
- Executes all tests in proper order
- Prompts for safety approval (ESP300, MaiTai)
- Handles timeouts gracefully
- Logs all output with timestamps
- Time: 6-7 hours

**Script 3: verify_hardware.sh**
- Pre-test hardware verification
- Checks VISA library
- Checks serial ports
- Checks camera
- Checks permissions
- Checks disk space
- Time: 2 minutes

**Script 4: post_test_analysis.sh**
- Analyzes test results
- Generates summary
- Checks for regressions
- Provides final verdict
- Time: 10 minutes

**Best For**: Automation, reproducible testing, minimal manual intervention

---

## How to Use These Documents

### For Your First Time Testing

1. **Read** [HARDWARE_TESTING_SUMMARY.md](HARDWARE_TESTING_SUMMARY.md) (5 min)
   - Understand what you're about to do
   - Check if you have time and resources

2. **Verify** [HARDWARE_TEST_PREPARATION.md](HARDWARE_TEST_PREPARATION.md) - Part 1 (10 min)
   - SSH access verification
   - Hardware availability checklist

3. **Complete Safety Review** [HARDWARE_TEST_PREPARATION.md](HARDWARE_TEST_PREPARATION.md) - Part 3 (20 min)
   - Read all safety procedures
   - Print laser safety officer sign-off form

4. **Print** [HARDWARE_TEST_CHECKLIST.md](HARDWARE_TEST_CHECKLIST.md)
   - Print the checklist document
   - Have pen ready for checkmarks

5. **Execute** Using scripts or checklist
   - Option A: Run HARDWARE_TESTING_SCRIPTS.sh for automation
   - Option B: Follow HARDWARE_TEST_CHECKLIST.md manually

6. **Reference** [QUICK_START_HARDWARE_TESTING.md](QUICK_START_HARDWARE_TESTING.md)
   - Keep handy for quick lookups
   - Use for troubleshooting

### For Subsequent Testing

1. **Review** [HARDWARE_TESTING_SUMMARY.md](HARDWARE_TESTING_SUMMARY.md) (2 min)
2. **Quick Check** [QUICK_START_HARDWARE_TESTING.md](QUICK_START_HARDWARE_TESTING.md) (5 min)
3. **Execute** Using [HARDWARE_TESTING_SCRIPTS.sh](HARDWARE_TESTING_SCRIPTS.sh)
4. **Reference** [HARDWARE_VALIDATION_PLAN.md](HARDWARE_VALIDATION_PLAN.md) if issues arise

---

## Testing Timeline

```
WEEK OF 2025-11-17

MONDAY (Day 1): SCPI + Newport 1830-C
├─ 9:00-9:30 AM: Preparation & warm-up (30 min)
├─ 9:30-9:50 AM: SCPI tests (17 tests, 20 min)
├─ 9:50-10:10 AM: Newport tests (14 tests, 20 min)
└─ 10:10-10:30 AM: Results review (20 min)
Total: 2 hours, LOW RISK

TUESDAY (Day 2): PVCAM + ESP300
├─ 10:00-10:20 AM: Warm-up & setup (20 min)
├─ 10:20-10:50 AM: PVCAM tests (28 tests, 30 min)
├─ 10:50-11:10 AM: Results review (20 min)
├─ 1:00-1:20 PM: ESP300 safety briefing (20 min)
├─ 1:20-2:05 PM: ESP300 tests (16 tests, 45 min)
└─ 2:05-2:15 PM: Return to safe state (10 min)
Total: 3 hours, MEDIUM RISK

WEDNESDAY (Day 3): MaiTai Laser
├─ 2:00-2:30 PM: Laser Safety Officer briefing (30 min)
├─ 2:30-2:55 PM: Critical safety tests (25 min)
├─ 2:55-3:00 PM: Formal approval (5 min)
├─ 3:00-4:30 PM: MaiTai tests (19 tests, 1.5 hrs)
└─ 4:30-4:45 PM: Post-laser shutdown (15 min)
Total: 1.5 hours, CRITICAL RISK

THURSDAY: Post-Testing
├─ 10:00-10:30 AM: Results analysis (30 min)
├─ 10:30-10:45 AM: Regression testing (15 min)
└─ 10:45-11:00 AM: Git tagging & archiving (15 min)
Total: 1 hour

GRAND TOTAL: 7.5 hours over 4 days
```

---

## Critical Safety Information

### MaiTai Laser (Day 3)

**MANDATORY REQUIREMENTS**:
- [ ] Laser Safety Officer must be present
- [ ] Safety briefing must be completed
- [ ] Critical safety tests must pass
- [ ] Formal approval required before testing
- [ ] Shutter must start CLOSED
- [ ] Shutter must end CLOSED

**Safety Officer Sign-Off Form**: In HARDWARE_TEST_PREPARATION.md, Part 9

### Motion Control (Day 2, ESP300)

**MANDATORY CHECKS**:
- [ ] All axes must be homed before testing
- [ ] Soft limits must be set (-50 to +50 mm)
- [ ] Motion path must be clear
- [ ] Emergency stop must be tested
- [ ] Return axes to home after testing

### General Lab Safety

- [ ] Know emergency exits
- [ ] Know emergency stop button locations
- [ ] Have first aid kit accessible
- [ ] Know facility manager contact
- [ ] Understand evacuation procedures

---

## Test Distribution

| Actor | Tests | Duration | Risk | Supervisor |
|-------|-------|----------|------|-----------|
| SCPI | 17 | 20 min | Low | No |
| Newport 1830-C | 14 | 20 min | Low | No |
| PVCAM | 28 | 30 min | Medium | No |
| ESP300 | 16 | 45 min | Medium | Yes |
| MaiTai | 19 | 1.5 hrs | **Critical** | **Yes** |
| **Total** | **94** | **6-7 hrs** | | |

---

## Success Criteria

### Hardware Validation PASS Requirements (ALL must be met):

- [ ] 5/5 actors spawn without panic
- [ ] 94/94 tests pass
- [ ] Zero critical safety violations
- [ ] MaiTai shutter verified closed on startup and shutdown
- [ ] ESP300 axes returned to home position
- [ ] All mock tests still passing (no regressions)
- [ ] Complete test report generated

### Individual Actor Requirements:

**SCPI**: 17/17 tests pass
**Newport**: 14/14 tests pass
**PVCAM**: 28/28 tests pass
**ESP300**: 16/16 tests pass
**MaiTai**: 19/19 tests pass

---

## Troubleshooting

### Connection Issues
- SSH timeout → Check Tailscale VPN
- Port not found → Check `ls /dev/ttyUSB*`
- Permission denied → `sudo usermod -a -G dialout $USER`
- VISA not found → `sudo apt install ni-visa`

### Testing Issues
- Test hangs → `pkill -f "cargo run"`
- Timeout errors → Increase timeout value
- Hardware unresponsive → Power cycle, wait 15 seconds
- Camera not detected → Check USB 3.0 port

### Safety Issues
- Shutter won't close → STOP, contact facility manager
- Motion uncontrolled → Hit emergency stop immediately
- Laser unexpectedly open → Hit emergency stop
- Eye exposure → CALL 911

---

## Contact Information

```
FACILITY CONTACTS:
Laser Safety Officer: ___________________ Phone: __________
Facility Manager: ___________________ Phone: __________
Equipment Support: ___________________ Phone: __________
Emergency Services: 911

HARDWARE SUPPORT:
MaiTai (Spectra-Physics): ___________
Newport: ___________
PVCAM (Teledyne Photometrics): ___________
SCPI (Keysight): ___________
```

---

## Next Steps After Hardware Validation

1. **Review Results** (Day 4)
   - Analyze all test logs
   - Document any issues
   - Verify pass criteria met

2. **Regression Testing** (Day 4)
   - Confirm all mock tests still pass
   - No performance degradation
   - No new errors introduced

3. **Production Deployment** (Days 5-6)
   - Create systemd service
   - Setup monitoring
   - Deploy to production cluster

---

## Document Versions

| Document | Version | Date | Status |
|----------|---------|------|--------|
| HARDWARE_TESTING_SUMMARY.md | 1.0 | 2025-11-17 | Ready |
| HARDWARE_TEST_PREPARATION.md | 1.0 | 2025-11-17 | Ready |
| HARDWARE_TEST_CHECKLIST.md | 1.0 | 2025-11-17 | Ready |
| HARDWARE_VALIDATION_PLAN.md | 1.0 | 2025-11-17 | Ready |
| QUICK_START_HARDWARE_TESTING.md | 1.0 | 2025-11-17 | Ready |
| HARDWARE_TESTING_SCRIPTS.sh | 1.0 | 2025-11-17 | Ready |

---

## Getting Help

### During Testing

**Check**: [QUICK_START_HARDWARE_TESTING.md](QUICK_START_HARDWARE_TESTING.md) (emergency reference)

**Detailed Help**: [HARDWARE_VALIDATION_PLAN.md](HARDWARE_VALIDATION_PLAN.md) - Section 4.4 (troubleshooting)

**Safety Emergency**: Call emergency services (911)

### Before Testing

**Read**: [HARDWARE_TEST_PREPARATION.md](HARDWARE_TEST_PREPARATION.md) - Part 3 (safety procedures)

**Contact**: Laser Safety Officer for MaiTai approval

**Review**: [HARDWARE_VALIDATION_PLAN.md](HARDWARE_VALIDATION_PLAN.md) - Section 1 (hardware access)

---

## Document Status

**Created**: 2025-11-17
**Status**: Ready for Execution
**Next Review**: Every 2 weeks during hardware testing phase

---

**Ready to begin V4 hardware validation testing!**

Start with: [HARDWARE_TESTING_SUMMARY.md](HARDWARE_TESTING_SUMMARY.md)
