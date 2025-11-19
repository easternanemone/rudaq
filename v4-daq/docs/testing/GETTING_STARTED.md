# Getting Started: SSH Access and Remote Testing

New user guide to get SSH access and remote testing working in 15 minutes.

## Welcome

This guide will help you:
1. Set up SSH access to maitai-eos
2. Deploy code remotely
3. Run tests on hardware
4. Monitor and collect results

Estimated time: 15 minutes for complete setup.

## Prerequisites

You should have:
- Access to maitai-eos (ask your team lead)
- Tailscale VPN installed and connected
- Rust development environment on your local machine
- Basic familiarity with terminal/command line

## Part 1: SSH Setup (First Time Only - 10 minutes)

### Step 1a: Generate SSH Key

Open terminal and run:

```bash
# Generate key (press Enter when prompted for password)
ssh-keygen -t ed25519 -C "your-email@example.com" -f ~/.ssh/id_ed25519
```

You should see output like:
```
Your identification has been saved in /Users/you/.ssh/id_ed25519
Your public key has been saved in /Users/you/.ssh/id_ed25519.pub
```

### Step 1b: Add Key to SSH Agent

```bash
# Start the SSH agent
eval "$(ssh-agent -s)"

# Add your key
ssh-add ~/.ssh/id_ed25519

# Verify it was added
ssh-add -l
```

### Step 1c: Configure SSH

Create or edit `~/.ssh/config` file:

```bash
# On macOS/Linux
nano ~/.ssh/config
# or
vim ~/.ssh/config
```

Add this section at the end:

```ssh-config
Host maitai-eos
    HostName 100.91.139.XX
    User maitai
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 60
    ServerAliveCountMax 3
    Compression yes
```

**Important**: Replace `100.91.139.XX` with the actual Tailscale IP.

To find the IP:
```bash
tailscale status | grep maitai
```

Example output:
```
maitai-eos (100.91.139.42) linux; idle, tx 1234 rx 5678
```

Use `100.91.139.42` in your config.

### Step 1d: Test Connection

```bash
# Try to connect
ssh maitai-eos

# You should see something like:
# Welcome to maitai-eos
# maitai@maitai-eos:~$

# Disconnect
exit
```

**If it works**: Congratulations! SSH is set up.

**If it doesn't work**: See "Troubleshooting" section below.

## Part 2: Deploy Code (Every Test - 2 minutes)

### Step 2a: Navigate to Project

```bash
cd ~/code/rust-daq/v4-daq
# or wherever you cloned the project
```

### Step 2b: Deploy Using Script

```bash
# Deploy to maitai-eos
./scripts/remote/deploy_to_maitai.sh
```

You should see:
```
[SUCCESS] SSH connection OK
[SUCCESS] Source code synced
[SUCCESS] Remote build check passed
[SUCCESS] Deployment completed successfully!
```

This script:
- Checks your SSH connection
- Copies code to remote system
- Verifies everything is ready
- Creates a deployment log

**Time**: 1-5 minutes depending on code size.

## Part 3: Run Tests (Every Test - 5+ minutes)

### Step 3a: Run Tests

```bash
# Run all tests
./scripts/remote/run_tests_remote.sh
```

Or for specific test types:

```bash
# Just unit tests (fastest)
./scripts/remote/run_tests_remote.sh --suite lib

# Just integration tests
./scripts/remote/run_tests_remote.sh --suite integration

# Hardware tests
./scripts/remote/run_tests_remote.sh --suite hardware

# Release mode (faster execution, slower compile)
./scripts/remote/run_tests_remote.sh --release
```

### Step 3b: Watch Progress (Optional)

In a separate terminal window:

```bash
# Monitor test progress in real-time
./scripts/remote/monitor_tests.sh
```

You'll see:
- Live test count
- Progress bar
- Failed tests as they happen
- System resource usage

### Step 3c: Results

Tests save results automatically to:
```
./test_results/YYYYMMDD_HHMMSS/
```

For example:
```
./test_results/20251118_071234/
├── manifest.txt        <- Test metadata
├── test_output.log     <- Full test output
└── results/            <- Individual test files
```

View results:
```bash
# See latest results
ls -lah ./test_results/

# View test output
cat ./test_results/*/test_output.log

# See summary
cat ./test_results/*/manifest.txt
```

## Quick Workflow

Once set up, your typical workflow is:

```bash
# 1. Make code changes
vim src/main.rs

# 2. Deploy
./scripts/remote/deploy_to_maitai.sh

# 3. Test
./scripts/remote/run_tests_remote.sh

# 4. Check results
cat ./test_results/*/test_output.log
```

**Total time**: 5-30 minutes depending on changes and test suite.

## Common Commands

### Deploy code
```bash
./scripts/remote/deploy_to_maitai.sh
```

### Run tests
```bash
./scripts/remote/run_tests_remote.sh              # All tests
./scripts/remote/run_tests_remote.sh --suite lib  # Unit tests only
./scripts/remote/run_tests_remote.sh --release    # Release mode
```

### Monitor progress
```bash
./scripts/remote/monitor_tests.sh
```

### SSH directly
```bash
ssh maitai-eos                      # Connect
ssh maitai-eos 'cargo test'        # Run test
exit                                # Disconnect
```

### Check test results
```bash
ls -lah ./test_results/
cat ./test_results/*/test_output.log
```

## Troubleshooting

### Problem: "ssh: command not found"

SSH should be built-in. Try:
```bash
which ssh
ssh -V  # version
```

### Problem: "Permission denied (publickey)"

Your public key wasn't copied. Do this:

```bash
# Copy your key to remote (one time)
ssh-copy-id -i ~/.ssh/id_ed25519.pub maitai@100.91.139.XX
# (Replace IP with actual Tailscale IP)

# Then test
ssh maitai-eos 'echo Connected'
```

### Problem: "Name or service not known"

Tailscale not connected. Check:
```bash
# Is Tailscale running?
tailscale status

# Should show maitai-eos as available
# If not: open Tailscale app and enable
```

### Problem: "Connection refused"

SSH not running on remote. Try:
```bash
# Test network
ping 100.91.139.XX -c 3

# Verbose ssh
ssh -vvv maitai-eos
```

### Problem: Scripts won't run

Make executable:
```bash
chmod +x ./scripts/remote/*.sh

# Then try again
./scripts/remote/deploy_to_maitai.sh
```

### Problem: "Cargo: command not found" (on remote)

Rust not installed. Ask admin to install or follow:
```bash
ssh maitai-eos << 'EOF'
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustc --version
EOF
```

### Problem: Tests hang

Set a timeout:
```bash
./scripts/remote/run_tests_remote.sh --timeout 300
# (300 seconds = 5 minutes)
```

### Problem: "No space left on device"

Remote disk is full. Clean up:
```bash
ssh maitai-eos 'cd ~/rust-daq && cargo clean'
```

## Getting Help

### For SSH issues
See: `docs/testing/SSH_ACCESS_GUIDE.md`
- Complete setup instructions
- 6+ specific troubleshooting cases
- Security best practices

### For testing issues
See: `docs/testing/REMOTE_TESTING_GUIDE.md`
- All test scenarios
- Workflow examples
- Real-time monitoring
- Result collection

### For file transfer issues
See: `docs/testing/FILE_TRANSFER_GUIDE.md`
- Multiple sync methods
- Performance optimization
- Troubleshooting procedures

### Quick reference
See: `docs/testing/QUICK_REFERENCE.md`
- Print-friendly command card
- Copy-paste ready commands
- Emergency procedures

## Next Steps

1. **Complete SSH setup** (Steps 1a-1d above)
2. **Test deployment** (Step 2b above)
3. **Run first test** (Step 3a above)
4. **Keep QUICK_REFERENCE.md handy** for daily use
5. **Read full guides** as needed for advanced usage

## Key Points to Remember

1. **Always check Tailscale is connected**
   ```bash
   tailscale status
   ```

2. **Deploy before testing**
   ```bash
   ./scripts/remote/deploy_to_maitai.sh
   ```

3. **Use release mode for performance tests**
   ```bash
   ./scripts/remote/run_tests_remote.sh --release
   ```

4. **Results are timestamped**
   ```bash
   ls ./test_results/
   ```

5. **Scripts have help**
   ```bash
   ./scripts/remote/deploy_to_maitai.sh --help
   ./scripts/remote/run_tests_remote.sh --help
   ./scripts/remote/monitor_tests.sh --help
   ```

## Common Questions

**Q: Do I need to set up SSH every time?**
A: No, just once. After initial setup, only use the deployment and test scripts.

**Q: Can I run multiple tests in parallel?**
A: Yes, but best to run one at a time. You can monitor in separate terminal.

**Q: How long do tests take?**
A: Unit tests: 1-3 min. Integration: 5-15 min. Hardware: 10-30+ min.

**Q: Where are test results stored?**
A: In `./test_results/<timestamp>/` with full logs and metadata.

**Q: Can I customize test options?**
A: Yes! See `run_tests_remote.sh --help` for all options.

**Q: What if I need to debug a test?**
A: Use `--suite lib` for quick feedback, or `--no-download` to watch remotely.

## One-Minute Summary

```bash
# Setup (one time)
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519
ssh-add ~/.ssh/id_ed25519
# Edit ~/.ssh/config (see Step 1c)
ssh maitai-eos  # verify

# Every test
./scripts/remote/deploy_to_maitai.sh  # Deploy
./scripts/remote/run_tests_remote.sh   # Test
cat ./test_results/*/test_output.log   # Results
```

## Where to Go From Here

### For beginners
1. Finish this guide
2. Read `SSH_ACCESS_GUIDE.md` (complete understanding)
3. Try different test suites with `run_tests_remote.sh`

### For advanced users
1. See `REMOTE_TESTING_GUIDE.md` for workflows
2. See `FILE_TRANSFER_GUIDE.md` for optimization
3. See `QUICK_REFERENCE.md` for emergency commands

### For troubleshooting
1. Check `QUICK_REFERENCE.md` first
2. See relevant guide section
3. Run diagnostic command
4. Check log files

## You're Ready!

You now have everything you need to:
- Access maitai-eos via SSH
- Deploy code remotely
- Run hardware tests
- Monitor progress
- Collect results

Next step: Follow Part 1, Part 2, and Part 3 above to get started!

Questions? Check the relevant guide document or ask your team.

Happy testing!
