# Complete Documentation Index

## Core Documentation (for SSH & Remote Testing)

### 1. README.md (352 lines)
**Start here for overview**
- Quick start guide (5 minutes)
- Documentation structure
- Available scripts with examples
- Common workflows
- Troubleshooting overview
- Performance tips

**Key Sections:**
- Quick Start
- Common Workflows
- SSH Configuration
- File Organization

### 2. SSH_ACCESS_GUIDE.md (475 lines)
**Complete SSH setup and troubleshooting**
- SSH key generation and setup
- Tailscale VPN configuration
- Initial connection testing
- SSH config file setup
- Port forwarding instructions
- Troubleshooting 6+ SSH issues
- Security best practices
- Advanced SSH features (tunnels, key rotation)
- Batch SSH commands
- Session persistence with tmux
- Automated connection testing

**Key Sections:**
- Step 1: SSH Key Setup
- Step 2: Tailscale Configuration
- Step 3: SSH Config File Setup
- Step 4: Initial Connection Test
- Step 5: Troubleshooting SSH Issues
- Security Best Practices
- Quick Reference Commands

### 3. REMOTE_TESTING_GUIDE.md (587 lines)
**Remote testing procedures and workflows**
- Code deployment strategies (script vs manual)
- Building on remote system
- Running various test suites
- Real-time test output capture
- Collecting test results
- Handling SSH disconnections
- Real-time test monitoring
- Complete workflow examples
- Troubleshooting test issues
- Performance optimization

**Key Sections:**
- Part 1: Deploying Code
- Part 2: Building on Remote System
- Part 3: Running Tests Remotely
- Part 4: Collecting Test Results
- Part 5: Handling SSH Disconnections
- Part 6: Real-Time Monitoring
- Workflow Examples (4 examples)
- Troubleshooting

### 4. FILE_TRANSFER_GUIDE.md (524 lines)
**Efficient file transfer strategies**
- SCP for simple transfers
- rsync for incremental syncing
- git-based synchronization
- tar for batch transfers
- Automated sync scripts
- Network optimization
- Troubleshooting file transfers
- Performance comparisons

**Key Sections:**
- Method 1: SCP (Simple Copy)
- Method 2: rsync (Recommended)
- Method 3: Git
- Method 4: tar
- Automating File Transfers
- Network Optimization
- Troubleshooting

### 5. QUICK_REFERENCE.md (321 lines)
**Fast lookup card - print and keep nearby**
- SSH setup one-liner
- Deploy code commands
- Run tests commands
- Monitor tests commands
- File transfer commands
- Remote commands
- Persistent sessions (tmux)
- Troubleshooting quick commands
- Common workflows (one-liners)
- Test results location
- Environment variables
- Documentation table
- SSH config template
- Emergency commands

**Perfect for:**
- Quick lookup while working
- Copy-paste commands
- Emergency procedures
- Reference card at desk

## Automation Scripts (in scripts/remote/)

### 6. deploy_to_maitai.sh (247 lines)
**Deploy code to maitai-eos with verification**

Usage:
```bash
./scripts/remote/deploy_to_maitai.sh          # Standard deployment
./scripts/remote/deploy_to_maitai.sh --full   # Full sync
./scripts/remote/deploy_to_maitai.sh --no-build # Skip build check
```

What it does:
- Checks SSH connection
- Verifies prerequisites
- Syncs source code via rsync
- Verifies remote files
- Checks Rust toolchain
- Runs remote build verification
- Generates deployment report

Output:
- `deploy_<timestamp>.log` - Full deployment log

### 7. run_tests_remote.sh (267 lines)
**Run tests remotely and download results**

Usage:
```bash
./scripts/remote/run_tests_remote.sh                              # All tests
./scripts/remote/run_tests_remote.sh --suite lib                  # Unit tests
./scripts/remote/run_tests_remote.sh --suite hardware --release   # Hardware in release
./scripts/remote/run_tests_remote.sh --threads 4 --timeout 600    # Custom config
```

Options:
- `--suite`: all, lib, integration, hardware
- `--release`: Release mode compilation
- `--no-capture`: Show println! output
- `--threads N`: Set test thread count
- `--timeout SECONDS`: Maximum test duration
- `--no-download`: Don't download results locally

Output:
- `./test_results/<timestamp>/test_output.log`
- `./test_results/<timestamp>/manifest.txt`
- `./test_results/<timestamp>/results/` (if available)

### 8. monitor_tests.sh (185 lines)
**Real-time test monitoring dashboard**

Usage:
```bash
./scripts/remote/monitor_tests.sh
```

Shows:
- Live test count and progress
- Progress bar with percentage
- Elapsed time and ETA
- Failed tests as they happen
- System resource usage (disk, memory)
- Cargo process count

Features:
- Auto-updates every 5 seconds
- Colored output for easy reading
- Shows failures in real-time
- Monitors system health

## File Organization

```
docs/testing/
├── INDEX.md                           <- You are here
├── README.md                          <- Start here
├── SSH_ACCESS_GUIDE.md               <- Initial SSH setup
├── REMOTE_TESTING_GUIDE.md           <- Testing procedures
├── FILE_TRANSFER_GUIDE.md            <- File sync strategies
└── QUICK_REFERENCE.md                <- Lookup card

scripts/remote/
├── deploy_to_maitai.sh               <- Deploy code
├── run_tests_remote.sh               <- Run tests
└── monitor_tests.sh                  <- Watch progress

Supporting docs (existing):
├── HARDWARE_TEST_CHECKLIST.md        <- Hardware test verification
├── HARDWARE_TEST_PREPARATION.md      <- Hardware setup
├── HARDWARE_VALIDATION_PLAN.md       <- Complete validation strategy
├── QUICK_START_HARDWARE_TESTING.md   <- Hardware testing quickstart
└── README_HARDWARE_TESTING.md        <- Hardware testing overview
```

## Document Relationships

```
First Time User:
  1. SSH_ACCESS_GUIDE.md (Setup SSH)
  2. deploy_to_maitai.sh (Deploy code)
  3. REMOTE_TESTING_GUIDE.md (Understand testing)
  4. run_tests_remote.sh (Run first test)
  5. QUICK_REFERENCE.md (Keep for reference)

Regular User:
  1. QUICK_REFERENCE.md (Copy commands)
  2. deploy_to_maitai.sh (Deploy)
  3. run_tests_remote.sh (Test)
  4. monitor_tests.sh (Watch)

Problem Solving:
  1. QUICK_REFERENCE.md (Quick troubleshooting)
  2. Relevant section from main guides
  3. SSH_ACCESS_GUIDE.md Section 5 (for SSH issues)
  4. REMOTE_TESTING_GUIDE.md Troubleshooting (for test issues)
  5. FILE_TRANSFER_GUIDE.md Troubleshooting (for file issues)
```

## Quick Navigation

### "I need to..."

| Need | Document | Section |
|------|----------|---------|
| Set up SSH for first time | SSH_ACCESS_GUIDE.md | Steps 1-4 |
| Connect to maitai-eos | SSH_ACCESS_GUIDE.md | Step 4 |
| Fix "Permission denied" error | SSH_ACCESS_GUIDE.md | Issue: Permission denied |
| Deploy code | deploy_to_maitai.sh | Run script |
| Run tests | run_tests_remote.sh | Run script |
| Monitor tests | monitor_tests.sh | Run script |
| Find test results | README.md | Test Results Storage |
| Transfer files | FILE_TRANSFER_GUIDE.md | Methods 1-4 |
| Make rsync faster | FILE_TRANSFER_GUIDE.md | Network Optimization |
| Fix SSH timeout | SSH_ACCESS_GUIDE.md | Issue: Timeout |
| Learn tmux | REMOTE_TESTING_GUIDE.md | Using tmux |
| Find all commands | QUICK_REFERENCE.md | All sections |

## Statistics

| Category | Count | Details |
|----------|-------|---------|
| Documentation Files | 5 | SSH, Testing, Files, Quick Ref, Index |
| Guide Lines | 2,063 | Total documentation lines |
| Scripts | 3 | Deploy, Test, Monitor |
| Script Lines | 699 | Total script lines |
| Total Lines | 2,762 | Complete reference |

## Print-Friendly Guide

For printing, recommend:
1. **QUICK_REFERENCE.md** - Single page cheat sheet (keep at desk)
2. **SSH_ACCESS_GUIDE.md** - Initial setup reference
3. **REMOTE_TESTING_GUIDE.md** - Common workflows

## Regular Workflow (Once Set Up)

After initial SSH setup (done once):

```bash
# Every test run:
./scripts/remote/deploy_to_maitai.sh  # Deploy latest
./scripts/remote/run_tests_remote.sh   # Run tests
./scripts/remote/monitor_tests.sh      # Watch (in separate terminal)
```

Expected time: 2-10 minutes depending on changes and test suite.

## Troubleshooting Decision Tree

```
Can't connect to maitai-eos?
├─ Check SSH_ACCESS_GUIDE.md Section 5
├─ Run: ssh -vvv maitai-eos
└─ Check Tailscale: tailscale status

Tests fail?
├─ Check REMOTE_TESTING_GUIDE.md Troubleshooting
├─ Run: ssh maitai-eos 'cargo test --lib'
└─ Check logs: cat test_results/*/test_output.log

Files won't sync?
├─ Check FILE_TRANSFER_GUIDE.md Troubleshooting
├─ Run dry-run: rsync -avz --dry-run
└─ Check disk: ssh maitai-eos 'df -h'

Script doesn't work?
├─ Check QUICK_REFERENCE.md Emergency Commands
├─ Verify: ssh maitai-eos 'uname -a'
└─ Clean: ssh maitai-eos 'cd ~/rust-daq && cargo clean'
```

## Getting Help

1. Check relevant document first
2. Try QUICK_REFERENCE.md troubleshooting
3. Search specific guide for your issue
4. Review error message carefully
5. Try running diagnostic from your document
6. Check log files in `./test_results/`

## Document Maintenance

These documents should be updated when:
- SSH/Tailscale configuration changes
- New test scripts are added
- Common issues arise that aren't documented
- Network setup changes
- Remote system changes

## Version Information

- Created: 2025-11-18
- SSH: OpenSSH (latest)
- Rust: 1.70+ (as installed on maitai-eos)
- OS: Ubuntu/Linux (maitai-eos)
- VPN: Tailscale (latest)

## Next Steps

1. Start with README.md for overview
2. Go to SSH_ACCESS_GUIDE.md for setup
3. Run deploy_to_maitai.sh to verify
4. Use QUICK_REFERENCE.md for daily work
5. Bookmark this INDEX for navigation
