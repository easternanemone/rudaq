# Hardware Validation - Quick Start Guide

Fast-track guide to running hardware validation tests safely.

## TL;DR - The 30-Second Version

```bash
# 1. Verify everything is ready
./verify_hardware.sh --quick

# 2. Check safety (answer a few questions)
./safety_check.sh

# 3. Run tests
./run_all_tests.sh

# 4. Review results
./analyze_results.sh
```

All scripts are in: `scripts/hardware_validation/`

## Before You Start

**Checklist:**
- [ ] MaiTai shutter is CLOSED (critical!)
- [ ] Lab area is clear
- [ ] Emergency stop is accessible
- [ ] SSH key is set up for maitai@maitai-eos
- [ ] You have ~3 hours available

## Running Tests

### Option 1: Interactive (Recommended)

```bash
cd scripts/hardware_validation/
./run_all_tests.sh
```

The script will:
- Ask before each phase
- Show progress
- Stop if something fails
- Save detailed logs

### Option 2: Automated (Hands-Off)

```bash
./run_all_tests.sh --auto
```

Runs to completion without asking. Still safe - will stop on errors.

### Option 3: Resume After Interruption

```bash
./run_all_tests.sh --resume esp300
```

Skips SCPI and Newport, starts at ESP300.

## Understanding the Output

### Colors Mean:
- **Green `[PASS]`** - Test passed successfully
- **Red `[ERROR]`** - Test failed, stopping
- **Yellow `[WARN]`** - Warning, but continuing
- **Blue `[INFO]`** - Status information

### Progress Indicators:
```
[20%] Running Newport motion control tests
[PASS] Newport tests passed
[INFO] Newport phase completed in 1234s
```

## If Something Goes Wrong

### Tests Fail
```bash
# Check detailed logs
cat hardware_test_logs/[phase]_*.log

# Generate analysis
./analyze_results.sh

# This creates GitHub issues automatically
./analyze_results.sh --issues
```

### Emergency Stop
```bash
# Press Ctrl+C in the terminal running tests

# Or in another terminal:
./emergency_stop.sh --force
```

## Test Phases Explained

| Phase | Risk | Time | What |
|-------|------|------|------|
| SCPI | Low | 20 min | Basic communication test |
| Newport | Low | 20 min | Stage movement test |
| ESP300 | Medium | 45 min | Motor controller test |
| PVCAM | Medium | 30 min | Camera test |
| MaiTai | Critical | 90 min | Laser test (LSO approval needed) |

## For MaiTai Laser Testing

The script will ask for:
1. **Laser Safety Officer approval** - Type their name
2. **Shutter confirmation** - Type "yes" that it's closed

Then it runs with a 2-minute timeout for safety.

## Log Files Location

Everything goes to: `hardware_test_logs/`

**Key files:**
- `test_report_[timestamp].txt` - Main results
- `test_summary_[timestamp].json` - Metrics (for tracking)
- `[phase]_[timestamp].log` - Detailed errors per phase
- `safety_check_[timestamp].log` - Safety verification audit

## Common Issues & Fixes

**"Cannot connect to maitai@maitai-eos"**
```bash
# Check SSH key
ls ~/.ssh/id_rsa ~/.ssh/id_ed25519

# Test connection
ssh maitai@maitai-eos "echo test"
```

**"No serial ports detected"**
```bash
# List serial ports
ls -la /dev/tty*

# May need permission fix
sudo usermod -a -G dialout $USER
newgrp dialout
```

**"Tests fail to compile"**
```bash
# Clean rebuild
cargo clean
cargo build --release --tests
```

## Success Indicators

**After test completion, you'll see:**
```
════════════════════════════════════════════════════════════
                   Test Execution Report
════════════════════════════════════════════════════════════

Phase Results:
  ✓ scpi: PASS
  ✓ newport: PASS
  ✓ esp300: PASS
  ✓ pvcam: PASS
  ✓ maitai: PASS

Summary:
  Total Phases: 5
  Passed: 5
  Failed: 0
  Success Rate: 100%
```

## Next Steps

1. **Archive successful results**
   ```bash
   cp hardware_test_logs/test_summary_*.json baseline_metrics.json
   git add hardware_test_logs/
   git commit -m "Hardware validation passed"
   ```

2. **For future runs, compare against baseline**
   ```bash
   ./analyze_results.sh --baseline baseline_metrics.json
   ```

3. **If there are failures**
   ```bash
   ./analyze_results.sh --issues
   # Review created GitHub issues
   ```

## Full Command Reference

```bash
# Verification & Safety
./verify_hardware.sh              # Full hardware check
./verify_hardware.sh --quick      # Quick connectivity only
./safety_check.sh                 # Safety verification
./safety_check.sh --pre-maitai    # Critical pre-laser checks

# Test Execution
./run_all_tests.sh                # Interactive (default)
./run_all_tests.sh --auto         # Non-interactive
./run_all_tests.sh --resume esp300  # Resume from phase

# Analysis & Reporting
./analyze_results.sh              # Analyze latest results
./analyze_results.sh --issues     # Generate GitHub issues
./analyze_results.sh --baseline baseline.json  # Compare

# Emergency
./emergency_stop.sh               # Emergency stop (confirm)
./emergency_stop.sh --force       # Force immediate stop
```

## Estimated Timeline

```
Preparation:
  verify_hardware.sh     ~5 min
  safety_check.sh        ~5 min
  Total prep:           10 min

Testing:
  SCPI                  20 min
  Newport               20 min
  Safety check          ~2 min
  ESP300                45 min
  PVCAM                 30 min
  Safety check          ~2 min
  MaiTai                90 min
  Total testing:       ~3 hours

Analysis:
  analyze_results.sh     ~2 min
  Total:                ~3 hours 12 minutes
```

## Getting Help

Each script has built-in help:
```bash
./run_all_tests.sh --help
./verify_hardware.sh --help
./safety_check.sh --help
./analyze_results.sh --help
./emergency_stop.sh --help
```

For more details, see: `README.md` in same directory.
