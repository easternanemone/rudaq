# Hardware Validation Test Suite

Comprehensive bash automation for hardware testing and validation with safety checks, progress monitoring, and detailed reporting.

## Overview

This test suite provides automated execution of hardware tests in a safe, controlled manner:

- **Phased Testing**: Tests run in sequence from safest to riskiest
- **Safety Checkpoints**: Critical safety verifications before each phase
- **Interactive Controls**: Pause and confirm before proceeding
- **Detailed Logging**: Complete audit trail of all operations
- **Result Analysis**: Automatic metrics calculation and issue generation
- **Emergency Stop**: Immediate halt capability for any phase

## Script Files

### 1. `run_all_tests.sh` - Master Test Runner

Orchestrates the complete test execution sequence with safety checkpoints.

**Usage:**
```bash
# Interactive mode (default - waits for confirmation at each checkpoint)
./run_all_tests.sh

# Non-interactive mode (automatic progression)
./run_all_tests.sh --auto

# Resume from specific phase
./run_all_tests.sh --resume esp300
```

**Test Phases:**
1. **SCPI Tests** (LOW RISK, 20 min)
   - Basic SCPI device communication
   - No motor movement or laser

2. **Newport 1830-C** (LOW RISK, 20 min)
   - Stage movement within limits
   - Limited range testing

3. **ESP300** (MEDIUM RISK, 45 min)
   - Motor controller testing
   - Multi-axis motion
   - Safety limits verification

4. **PVCAM** (MEDIUM RISK, 30 min)
   - Camera acquisition
   - Image processing
   - Frame rate testing

5. **MaiTai** (CRITICAL RISK, 90 min)
   - Laser power measurement
   - Wavelength tuning
   - **Requires Laser Safety Officer approval**
   - **Requires all safety interlocks**

**Output:**
- Timestamped test results in `hardware_test_logs/`
- Summary JSON file
- Complete execution log
- Per-phase detailed logs

**Safety Features:**
```
Set -euo pipefail for robustness
Color-coded output for readability
Interactive safety confirmations
SSH connectivity validation
Soft limit verification
Emergency stop accessible
```

### 2. `verify_hardware.sh` - Hardware Verification

Verifies all hardware prerequisites before running tests.

**Usage:**
```bash
# Full verification (default)
./verify_hardware.sh

# Quick connectivity check only
./verify_hardware.sh --quick

# Detailed output
./verify_hardware.sh --verbose
```

**Verification Checks:**

**SSH & Network:**
- SSH connectivity to maitai@maitai-eos
- SSH key setup and permissions
- Network interface availability
- General internet connectivity
- maitai-eos network reachability

**VISA Resources:**
- VISA library availability
- VISA device enumeration
- LD_LIBRARY_PATH configuration

**Hardware Detection:**
- Serial port enumeration
- PVCAM library detection
- Camera device permissions
- Video device detection

**System Checks:**
- Rust/Cargo installation
- Project compilation status
- Disk space availability (requires 1GB)

**Output:**
- Verification report with pass/fail status
- Warnings for non-critical issues
- Recommendations for fixes
- Log file: `verify_report_[timestamp].txt`

### 3. `safety_check.sh` - Safety Verification

Critical safety checks before hardware testing.

**Usage:**
```bash
# Full safety check (interactive)
./safety_check.sh

# Pre-MaiTai critical checks only
./safety_check.sh --pre-maitai

# Automated mode (no user prompts)
./safety_check.sh --automated
```

**Safety Checks:**

**MaiTai Laser:**
- Shutter state verification
- **MUST be CLOSED** - failure aborts test
- Laser Safety Officer approval required
- Lab safety checklist confirmation

**ESP300 Motor Controller:**
- Soft limits verification
- Serial port detection
- Device responsiveness check

**General Safety:**
- Emergency stop accessibility
- Emergency stop procedure acknowledgment
- Lab area clearance
- Equipment grounding
- Fire suppression availability
- Safety equipment availability

**Lab Checklist:**
- Personnel authorization
- Grounding verification
- Fire extinguisher access
- First aid kit availability
- Safety goggles availability
- Laser warning signs posted
- Emergency contacts posted
- Hazardous materials labeled

**Output:**
- Pass/fail status for each check
- LSO approval recorded with name and timestamp
- Safety log file: `safety_check_[timestamp].log`
- Detailed error messages

### 4. `analyze_results.sh` - Result Analysis

Parses test outputs and generates comprehensive reports.

**Usage:**
```bash
# Analyze latest test results
./analyze_results.sh

# Analyze specific report
./analyze_results.sh --report hardware_test_logs/test_report_20251118_071000.txt

# Compare against baseline
./analyze_results.sh --baseline baseline_metrics.json

# Generate GitHub issues for failures
./analyze_results.sh --issues

# Combined: compare and generate issues
./analyze_results.sh --baseline baseline.json --issues
```

**Metrics Calculated:**
- Total tests run per phase
- Pass/fail counts
- Pass rate percentage
- Phase-by-phase breakdown
- Regression detection vs baseline
- Error categorization

**Baseline Comparison:**
- Passed tests delta (+/-)
- Failed tests delta (+/-)
- Pass rate delta (percentage points)
- Regression/improvement detection
- Detailed comparison output

**Output:**
- Analysis report: `analysis_[timestamp].txt`
- Metrics JSON: `metrics_[timestamp].json`
- GitHub issues folder: `github_issues_[timestamp]/`
- Summary printed to console

**GitHub Issue Format:**
- Automatic issue creation for each failure
- Links to detailed logs
- Severity assessment
- Labels and categorization
- Ready for `gh issue create` command

### 5. `emergency_stop.sh` - Emergency Procedures

Immediate stop of all hardware operations.

**Usage:**
```bash
# Interactive mode (asks for confirmation)
./emergency_stop.sh

# Force immediate stop (no confirmation)
./emergency_stop.sh --force
```

**Emergency Procedures:**
1. **MaiTai Laser** - Close shutter, disable output
2. **PVCAM Camera** - Terminate acquisition
3. **ESP300 Motor** - Stop all motion
4. **All Tests** - Kill all running processes
5. **Devices** - Disconnect all instruments

**Actions Taken:**
- Sends CLOSE command to MaiTai shutter via SSH
- Disables laser output
- Kills all `cargo test` processes
- Sends TTY stop signals to serial devices
- Logs all emergency actions
- Requests post-emergency manual verification

**Output:**
- Emergency log file: `emergency_[timestamp].log`
- Confirmation of each action
- Post-emergency checklist
- LSO contact information reminder

## Typical Workflows

### Complete Test Run

```bash
# 1. Verify hardware prerequisites
./verify_hardware.sh

# 2. Perform safety checks
./safety_check.sh

# 3. Run all tests
./run_all_tests.sh

# 4. Analyze results
./analyze_results.sh

# 5. Generate issues if needed
./analyze_results.sh --issues
```

### Resume After Interruption

```bash
# Check what tests ran
ls -lh hardware_test_logs/

# Resume from specific phase
./run_all_tests.sh --resume esp300

# Analyze final results
./analyze_results.sh
```

### Pre-MaiTai Laser Work

```bash
# Targeted safety check
./safety_check.sh --pre-maitai

# Run MaiTai tests only
./run_all_tests.sh --auto --resume maitai
```

### Emergency Response

```bash
# Immediate stop (force mode)
./emergency_stop.sh --force

# Or with confirmation
./emergency_stop.sh
```

## Directory Structure

```
scripts/
  hardware_validation/
    run_all_tests.sh          # Master test runner
    verify_hardware.sh        # Hardware verification
    safety_check.sh           # Safety checks
    analyze_results.sh        # Result analysis
    emergency_stop.sh         # Emergency procedures
    README.md                 # This file

hardware_test_logs/
  test_report_[timestamp].txt       # Main test report
  test_summary_[timestamp].json     # Summary JSON
  safety_check_[timestamp].log      # Safety log
  verify_report_[timestamp].txt     # Verification report
  analysis_[timestamp].txt          # Analysis report
  metrics_[timestamp].json          # Metrics JSON
  emergency_[timestamp].log         # Emergency log
  [phase]_[timestamp].log           # Per-phase logs
  github_issues_[timestamp]/        # Generated issues
```

## Key Features

### Safety First
- Colorized critical warnings
- Interactive confirmation prompts
- Soft limit verification
- Shutter state validation
- LSO approval recording
- Emergency stop capability

### Detailed Logging
- All operations logged with timestamps
- Per-phase execution logs
- Safety check audit trail
- Emergency event logging
- JSON metrics for analysis
- Git-friendly formats

### Test Ordering
```
Safest      ↓ Low Risk (40 min)
             SCPI & Newport
             ↓ Medium Risk (75 min)
             ESP300 & PVCAM
             ↓ Critical Risk (90 min)
Riskiest    MaiTai Laser
```

### Error Handling
- `set -euo pipefail` for safety
- Graceful failure handling
- Clear error messages
- Automatic recovery attempts
- Detailed error logging

### Progress Tracking
- Phase-by-phase execution
- Percentage progress indicators
- Real-time status messages
- Estimated duration per phase
- Total runtime calculation

## Safety Protocols

### MaiTai Laser Work

**Critical Requirements:**
1. Laser Safety Officer MUST be present
2. All safety interlocks MUST be engaged
3. MaiTai shutter MUST be CLOSED (verified)
4. Lab access must be restricted
5. Emergency stop must be accessible

**Procedure:**
1. `./safety_check.sh --pre-maitai` - Verify critical conditions
2. LSO provides approval (name recorded)
3. Shutter state verified (CLOSED)
4. Test run begins with timeout protection
5. All actions logged with LSO name

### Emergency Situations

**If Any Equipment Fails:**
1. Run `./emergency_stop.sh --force`
2. Verify all motion has stopped
3. Check MaiTai shutter is CLOSED
4. Contact Laser Safety Officer
5. Review emergency log

**If Tests Hang:**
```bash
# In another terminal:
./emergency_stop.sh --force

# Or manually:
pkill -9 -f "cargo test"
```

## Environment Requirements

**Operating System:**
- Linux or macOS with bash 4.0+
- SSH client (for maitai@maitai-eos)

**Hardware:**
- At least 1GB free disk space
- Network access to maitai-eos
- Serial port connectivity (for some devices)

**Software:**
- Rust/Cargo (for test compilation)
- VISA libraries (for instrument communication)
- Python (optional, for PyVISA device enumeration)

**Configuration:**
- SSH key setup for maitai@maitai-eos
- VISA resource configuration
- Serial port permissions
- Cargo project in good state

## Configuration Files

### Environment Variables

```bash
# Control logging verbosity
export HARDWARE_TEST_VERBOSE=1

# Specify log directory
export HARDWARE_TEST_LOGS=/custom/log/path

# Set test timeout (seconds)
export HARDWARE_TEST_TIMEOUT=300
```

### SSH Configuration

Ensure `~/.ssh/config` has entry for maitai:
```
Host maitai-eos
    User maitai
    HostName maitai-eos.lab.local
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
```

## Troubleshooting

### SSH Connection Fails
```bash
# Check SSH key
ls -la ~/.ssh/id_rsa ~/.ssh/id_ed25519

# Test connection
ssh -vvv maitai@maitai-eos "echo test"

# Fix known_hosts if needed
ssh-keygen -R maitai-eos
```

### Serial Port Not Found
```bash
# List available ports
ls -la /dev/tty*

# Check permissions
sudo usermod -a -G dialout $USER

# Reload shell group membership
newgrp dialout
```

### VISA Libraries Not Found
```bash
# Set LD_LIBRARY_PATH
export LD_LIBRARY_PATH=/path/to/visa/lib:$LD_LIBRARY_PATH

# Or install VISA
# macOS: brew install ni-visa
# Linux: consult NI documentation
```

### Tests Fail to Compile
```bash
# Clean and rebuild
cargo clean
cargo build --release

# Check for issues
cargo check
```

## Performance Considerations

**Total Test Time:**
- SCPI: ~20 minutes
- Newport: ~20 minutes
- ESP300: ~45 minutes
- PVCAM: ~30 minutes
- MaiTai: ~90 minutes
- **Total: ~3 hours** (without overhead)

**Disk Space:**
- Test logs: ~50-100 MB per run
- Recommended: 1GB free

**Network:**
- SSH to maitai-eos
- Minimal bandwidth required
- Timeout: 5-10 seconds for critical operations

## Maintenance

### Creating a Baseline

```bash
# Run full test suite
./run_all_tests.sh

# Copy successful results as baseline
cp hardware_test_logs/test_summary_*.json baseline_metrics.json

# Future runs can compare against it
./analyze_results.sh --baseline baseline_metrics.json
```

### Archiving Results

```bash
# Create timestamped archive
tar -czf hardware_tests_$(date +%Y%m%d).tar.gz hardware_test_logs/

# Push to repository
git add hardware_test_logs/
git commit -m "Hardware test results [timestamp]"
git push
```

### Rotating Logs

```bash
# Keep only last 10 test runs
cd hardware_test_logs
ls -t | tail -n +11 | xargs rm -f
```

## Integration with CI/CD

### GitHub Actions Example

```yaml
- name: Hardware Verification
  run: |
    scripts/hardware_validation/verify_hardware.sh

- name: Run Hardware Tests
  run: |
    scripts/hardware_validation/run_all_tests.sh --auto

- name: Analyze Results
  if: always()
  run: |
    scripts/hardware_validation/analyze_results.sh --issues

- name: Upload Logs
  if: always()
  uses: actions/upload-artifact@v2
  with:
    name: hardware-test-logs
    path: hardware_test_logs/
```

## Support & Documentation

**Key Files:**
- `run_all_tests.sh` - Main orchestration script
- `verify_hardware.sh` - Pre-test validation
- `safety_check.sh` - Critical safety verification
- `analyze_results.sh` - Result metrics and reporting
- `emergency_stop.sh` - Emergency procedures

**Logs Location:**
- `hardware_test_logs/` - All test outputs

**Each Script:**
- Has `--help` option for detailed usage
- Includes verbose logging
- Creates timestamped artifacts
- Logs all critical decisions

## License

Part of the rust-daq v4 project. All scripts use standard bash without external dependencies.

## Questions?

Refer to individual script help:
```bash
./run_all_tests.sh --help
./verify_hardware.sh --help
./safety_check.sh --help
./analyze_results.sh --help
./emergency_stop.sh --help
```
