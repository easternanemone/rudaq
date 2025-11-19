# Remote Testing Guide for maitai-eos

This guide explains how to deploy rust-daq code to maitai-eos, build it remotely, run tests via SSH, and collect results locally.

## Prerequisites

- SSH access to maitai-eos configured (see `SSH_ACCESS_GUIDE.md`)
- Rust toolchain installed on maitai-eos
- Git installed on maitai-eos
- Your code changes committed locally

## Quick Start

For experienced users, the fastest way:

```bash
# Deploy to remote
./scripts/remote/deploy_to_maitai.sh

# Run tests
./scripts/remote/run_tests_remote.sh

# Monitor progress
./scripts/remote/monitor_tests.sh
```

Continue reading for detailed explanations.

## Part 1: Deploying Code to maitai-eos

### Method 1: Using Deployment Script (Recommended)

```bash
# From project root
./scripts/remote/deploy_to_maitai.sh
```

This script:
- Syncs source code to maitai-eos
- Verifies Rust toolchain
- Creates deployment log
- Reports status

### Method 2: Manual Deployment

#### Step 1: Create Remote Project Directory

```bash
# Connect to maitai-eos
ssh maitai-eos

# Create project directory if it doesn't exist
mkdir -p ~/rust-daq
exit
```

#### Step 2: Sync Source Code

```bash
# From your local machine
rsync -avz --delete \
  --exclude target \
  --exclude .git \
  --exclude Cargo.lock \
  ./ maitai-eos:~/rust-daq/

# This is slow. For faster syncing, see FILE_TRANSFER_GUIDE.md
```

#### Step 3: Verify Deployment

```bash
# SSH to remote
ssh maitai-eos

# Check files
cd ~/rust-daq
ls -la
git status    # Should show any local changes

# Check toolchain
rustc --version
cargo --version
```

### Deployment Strategy: Full vs Incremental

**Full Sync** (first time or major changes):
```bash
rsync -avz --delete ./ maitai-eos:~/rust-daq/
```

**Incremental Sync** (after small changes):
```bash
rsync -avz --delete src/ maitai-eos:~/rust-daq/src/
rsync -avz Cargo.toml maitai-eos:~/rust-daq/
rsync -avz Cargo.lock maitai-eos:~/rust-daq/
```

**Git-Based Sync** (if both have git history):
```bash
# On remote
ssh maitai-eos << 'EOF'
cd ~/rust-daq
git fetch origin main
git reset --hard origin/main
EOF
```

## Part 2: Building on Remote System

### Build the Entire Project

```bash
# Quick build (debug mode - fast compile, slow execution)
ssh maitai-eos 'cd ~/rust-daq && cargo build'

# Release build (slow compile, fast execution - better for tests)
ssh maitai-eos 'cd ~/rust-daq && cargo build --release'

# View output in real-time
ssh -t maitai-eos 'cd ~/rust-daq && cargo build 2>&1'
```

### Build Specific Targets

```bash
# Library only (faster)
ssh maitai-eos 'cd ~/rust-daq && cargo build --lib'

# Binary only
ssh maitai-eos 'cd ~/rust-daq && cargo build --bin myapp'

# With verbose output (for debugging)
ssh maitai-eos 'cd ~/rust-daq && cargo build --lib -vv'
```

### Check Build Status Without Building

```bash
# Quick syntax/type check (very fast, no compilation)
ssh maitai-eos 'cd ~/rust-daq && cargo check'

# Check all targets
ssh maitai-eos 'cd ~/rust-daq && cargo check --all-targets'
```

### Clean Build Cache

```bash
# Remove build artifacts (starts fresh)
ssh maitai-eos 'cd ~/rust-daq && cargo clean'

# Then rebuild
ssh maitai-eos 'cd ~/rust-daq && cargo build --release'
```

## Part 3: Running Tests Remotely

### List Available Tests

```bash
# See all tests without running them
ssh maitai-eos 'cd ~/rust-daq && cargo test --no-run --message-format=json 2>/dev/null' | grep '"name"'

# Or simpler:
ssh maitai-eos 'cd ~/rust-daq && cargo test -- --list'
```

### Run All Tests

```bash
# Basic test run
ssh maitai-eos 'cd ~/rust-daq && cargo test'

# With output (shows println! statements)
ssh maitai-eos 'cd ~/rust-daq && cargo test -- --nocapture'

# In release mode (much faster for performance tests)
ssh maitai-eos 'cd ~/rust-daq && cargo test --release'

# With verbose output
ssh maitai-eos 'cd ~/rust-daq && cargo test -- --nocapture --test-threads=1'
```

### Run Specific Test Suites

```bash
# Unit tests only
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib'

# Integration tests only
ssh maitai-eos 'cd ~/rust-daq && cargo test --test "*"'

# Doc tests
ssh maitai-eos 'cd ~/rust-daq && cargo test --doc'

# Single test file
ssh maitai-eos 'cd ~/rust-daq && cargo test --test integration_test'

# Tests matching pattern
ssh maitai-eos 'cd ~/rust-daq && cargo test measurement'

# Specific test function
ssh maitai-eos 'cd ~/rust-daq && cargo test measurement::test_conversion'
```

### Run Tests with Hardware

```bash
# For hardware integration tests
ssh maitai-eos 'cd ~/rust-daq && cargo test --test "*hardware*" -- --nocapture'

# With specific hardware connected
ssh maitai-eos 'cd ~/rust-daq && HW_TEST=1 cargo test --test elliptec_hardware_test'

# With timeout (useful for hanging tests)
timeout 300 ssh maitai-eos 'cd ~/rust-daq && cargo test --release -- --test-threads=1'
```

### Real-Time Test Output

Using SSH with `-t` flag for interactive output:

```bash
# See test output in real-time
ssh -t maitai-eos 'cd ~/rust-daq && cargo test 2>&1 | tee test_output.log'

# Better: use tmux session for persistent output
ssh maitai-eos 'tmux new-session -d -s tests'
ssh maitai-eos 'tmux send-keys -t tests "cd ~/rust-daq && cargo test --release" Enter'

# Watch progress
ssh maitai-eos 'tmux capture-pane -t tests -p'

# Or attach interactively
ssh -t maitai-eos 'tmux attach -t tests'
```

## Part 4: Collecting Test Results

### Download Test Output

```bash
# Copy single log file
scp maitai-eos:~/rust-daq/test_output.log ./test_results/

# Copy entire results directory
scp -r maitai-eos:~/rust-daq/results/ ./test_results/

# Copy with compression (faster over network)
ssh maitai-eos 'cd ~/rust-daq && tar czf results.tar.gz results/'
scp maitai-eos:~/rust-daq/results.tar.gz ./
tar xzf results.tar.gz
```

### Capture Test Output in Real-Time

```bash
# Method 1: tee on remote system
ssh maitai-eos 'cd ~/rust-daq && cargo test 2>&1 | tee test_output.log'

# Method 2: Use SSH pipeline
ssh maitai-eos 'cd ~/rust-daq && cargo test' > test_results.txt 2>&1

# Method 3: Background job with monitoring
ssh maitai-eos 'cd ~/rust-daq && cargo test > test.log 2>&1 &'
sleep 2
ssh maitai-eos 'tail -f ~/rust-daq/test.log'
```

### Parse Test Results

```bash
# Extract test summary
ssh maitai-eos 'cd ~/rust-daq && cargo test 2>&1' | grep -E "(test result:|passed|failed)"

# Count passed/failed tests
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib 2>&1' | tail -5

# Generate JSON report
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib -- --format=json' > test_report.json
```

### Create Timestamped Results

```bash
# Save results with timestamp
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
ssh maitai-eos 'cd ~/rust-daq && cargo test --release' > "results/test_${TIMESTAMP}.log" 2>&1

# Also copy any logs from remote
scp "maitai-eos:~/rust-daq/results/*.log" "results/${TIMESTAMP}/"
```

## Part 5: Handling SSH Disconnections

### Problem: SSH Session Times Out

SSH connections may be interrupted by network issues or timeouts.

**Solution 1: Use tmux for Session Persistence**

```bash
# Create persistent tmux session on remote
ssh maitai-eos 'tmux new-session -d -s cargo'

# Start test in tmux session
ssh maitai-eos 'tmux send-keys -t cargo "cd ~/rust-daq && cargo test --release" Enter'

# Disconnect and do other work
# Reattach later
ssh maitai-eos 'tmux attach -t cargo'

# Check status without attaching
ssh maitai-eos 'tmux capture-pane -t cargo -p'
```

**Solution 2: Use nohup**

```bash
# Run in background, immune to disconnections
ssh maitai-eos 'cd ~/rust-daq && nohup cargo test --release > test.log 2>&1 &'

# Check progress
sleep 60
ssh maitai-eos 'tail -n 50 ~/rust-daq/test.log'

# Download results when done
scp maitai-eos:~/rust-daq/test.log ./results/
```

**Solution 3: Increase SSH Timeout**

Update `~/.ssh/config`:
```ssh-config
Host maitai-eos
    ServerAliveInterval 30
    ServerAliveCountMax 10
    TCPKeepAlive yes
    # ... other settings ...
```

### Monitoring Long-Running Tests

```bash
# Method 1: Periodic status check
while true; do
    echo "$(date): $(ssh maitai-eos 'ps aux | grep cargo' | grep -v grep | wc -l) cargo processes"
    if ! ssh maitai-eos 'test -f /home/maitai/rust-daq/test.log'; then
        echo "Test not yet started"
    else
        echo "Last 5 lines:"
        ssh maitai-eos 'tail -5 /home/maitai/rust-daq/test.log'
    fi
    sleep 30
done

# Method 2: Watch with tmux
ssh -t maitai-eos 'tmux new-session -s monitor'
ssh maitai-eos 'tmux send-keys -t monitor "watch -n 5 \"tail -20 ~/rust-daq/test.log\"" Enter'
```

## Part 6: Real-Time Test Monitoring

### Using the Monitoring Script

```bash
# Start monitoring tests
./scripts/remote/monitor_tests.sh

# This shows:
# - Current test status
# - Progress bar
# - Failures in real-time
# - ETA for completion
```

### Manual Real-Time Monitoring

```bash
# Watch output as it's written
ssh maitai-eos 'tail -f ~/rust-daq/test.log'

# In separate terminal, watch progress
ssh maitai-eos 'watch -n 5 "ps aux | grep cargo"'

# Or combined view
ssh -t maitai-eos << 'EOF'
tmux new-session -d -s monitor
tmux split-window -h -t monitor
tmux send-keys -t monitor:0 "tail -f test.log" Enter
tmux send-keys -t monitor:1 "watch -n 5 'ps aux | grep cargo'" Enter
tmux attach -t monitor
EOF
```

## Workflow Examples

### Example 1: Simple Test Run

```bash
# 1. Deploy latest code
./scripts/remote/deploy_to_maitai.sh

# 2. Run unit tests
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib'

# 3. Run integration tests
ssh maitai-eos 'cd ~/rust-daq && cargo test --test "*"'

# 4. Download any results
scp maitai-eos:~/rust-daq/results/* ./test_results/
```

### Example 2: Hardware Integration Testing

```bash
# 1. Sync code
rsync -avz src/ maitai-eos:~/rust-daq/src/

# 2. Start test in tmux (persistent)
ssh maitai-eos 'tmux new-session -d -s hw_test'
ssh maitai-eos 'tmux send-keys -t hw_test "cd ~/rust-daq && HW_TEST=1 cargo test --test elliptec_hardware_test -- --nocapture" Enter'

# 3. Monitor progress
./scripts/remote/monitor_tests.sh

# 4. When done, download results
scp maitai-eos:~/rust-daq/hw_results.log ./
```

### Example 3: Release Build Testing

```bash
# Deploy code
./scripts/remote/deploy_to_maitai.sh

# Clean and build release
ssh maitai-eos << 'EOF'
cd ~/rust-daq
cargo clean
cargo build --release
cargo test --release -- --test-threads=1
EOF

# Save results
scp maitai-eos:~/rust-daq/target/release/deps/ ./results/release/
```

### Example 4: Continuous Testing Workflow

```bash
# Create test loop script
cat > test_loop.sh << 'EOF'
#!/bin/bash
while true; do
    echo "=== Test run at $(date) ==="
    ssh maitai-eos 'cd ~/rust-daq && cargo test --lib' | tee -a results.log

    # Count results
    PASSED=$(grep "test result: ok" results.log | wc -l)
    FAILED=$(grep "test result: FAILED" results.log | wc -l)

    echo "Cumulative: $PASSED passed, $FAILED failed"
    echo "Waiting 5 minutes for next run..."
    sleep 300
done
EOF

chmod +x test_loop.sh
./test_loop.sh
```

## Troubleshooting Remote Testing

### Issue: "ssh: connect to host maitai-eos port 22: Connection refused"

```bash
# Check if SSH is running on remote
ssh maitai-eos 'sudo systemctl status ssh'

# Check network connectivity
tailscale status | grep maitai
ping -c 3 100.91.139.XX

# Verify SSH config
ssh -G maitai-eos
```

### Issue: "Cargo: command not found"

```bash
# Check if Rust is installed on remote
ssh maitai-eos 'which cargo'
ssh maitai-eos 'rustc --version'

# If missing, install (as root)
ssh maitai-eos 'sudo apt-get install build-essential'
ssh maitai-eos 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh'
```

### Issue: "Permission denied: /home/maitai/rust-daq"

```bash
# Check directory ownership
ssh maitai-eos 'ls -la ~/rust-daq'

# Fix permissions if needed
ssh maitai-eos 'chmod -R u+rwx ~/rust-daq'

# Or remove and resync
ssh maitai-eos 'rm -rf ~/rust-daq'
./scripts/remote/deploy_to_maitai.sh
```

### Issue: Tests Hang or Timeout

```bash
# Run with timeout
timeout 300 ssh maitai-eos 'cd ~/rust-daq && cargo test'

# Or manually kill
ssh maitai-eos 'pkill -f cargo'

# Use single-threaded mode
ssh maitai-eos 'cd ~/rust-daq && cargo test -- --test-threads=1'
```

### Issue: Out of Disk Space

```bash
# Check disk usage
ssh maitai-eos 'df -h'

# Clean build artifacts
ssh maitai-eos 'cd ~/rust-daq && cargo clean'

# Remove old test results
ssh maitai-eos 'rm -rf ~/rust-daq/results/*.log'
```

## Performance Tips

### Faster Builds

```bash
# Use pre-built artifacts from CI
ssh maitai-eos 'cargo build --release --offline'

# Use incremental compilation
export CARGO_INCREMENTAL=1

# Limit parallel jobs if system is overloaded
cargo build -j 4

# Link with mold for faster linking
ssh maitai-eos 'RUSTFLAGS="-C link-arg=-fuse-ld=mold" cargo build --release'
```

### Faster Testing

```bash
# Run only changed tests
cargo test --lib -- --skip slow_integration_test

# Run in parallel
cargo test -- --test-threads=$(nproc)

# Run in release mode (slower compile, faster runtime)
cargo test --release
```

### Network Optimization

See `FILE_TRANSFER_GUIDE.md` for efficient rsync usage with:
- Compression
- Partial transfers
- Selective syncing

## Next Steps

1. See `SSH_ACCESS_GUIDE.md` for initial SSH setup
2. Use `FILE_TRANSFER_GUIDE.md` for efficient file transfers
3. Run `scripts/remote/deploy_to_maitai.sh` to start
4. Use `scripts/remote/run_tests_remote.sh` for testing
5. Use `scripts/remote/monitor_tests.sh` to watch progress
