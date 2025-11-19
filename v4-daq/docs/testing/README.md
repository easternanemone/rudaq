# Remote Testing Documentation

Complete guide for SSH access, file transfer, and remote testing on maitai-eos hardware system.

## Quick Start (5 minutes)

1. **Configure SSH** (first time only)
   ```bash
   # See SSH_ACCESS_GUIDE.md Step 1-4
   ssh maitai-eos
   exit
   ```

2. **Deploy code to remote**
   ```bash
   ./scripts/remote/deploy_to_maitai.sh
   ```

3. **Run tests**
   ```bash
   ./scripts/remote/run_tests_remote.sh
   ```

4. **Monitor progress**
   ```bash
   ./scripts/remote/monitor_tests.sh
   ```

## Documentation Structure

### For First-Time Setup
Start with **SSH_ACCESS_GUIDE.md**:
- SSH key generation and setup
- Tailscale VPN configuration
- Initial connection testing
- Troubleshooting SSH issues
- Security best practices

### For Deploying Code
See **REMOTE_TESTING_GUIDE.md**:
- Deploying code to maitai-eos
- Building on remote system
- Running specific test suites
- Handling long-running tests
- Real-time monitoring
- Collecting results

### For File Transfers
See **FILE_TRANSFER_GUIDE.md**:
- Using scp for simple transfers
- Using rsync for efficient syncing
- Using git for code updates
- Automated sync scripts
- Network optimization
- Troubleshooting transfers

## Available Scripts

All scripts are in `scripts/remote/`:

### deploy_to_maitai.sh
Deploy latest code to maitai-eos with verification.

```bash
# Standard deployment
./scripts/remote/deploy_to_maitai.sh

# Full sync (includes everything)
./scripts/remote/deploy_to_maitai.sh --full

# Skip build verification
./scripts/remote/deploy_to_maitai.sh --no-build
```

### run_tests_remote.sh
Run tests on maitai-eos and download results.

```bash
# Run all tests
./scripts/remote/run_tests_remote.sh

# Run only unit tests
./scripts/remote/run_tests_remote.sh --suite lib

# Run release mode (faster)
./scripts/remote/run_tests_remote.sh --release

# Run specific hardware tests
./scripts/remote/run_tests_remote.sh --suite hardware

# Run with 4 test threads
./scripts/remote/run_tests_remote.sh --threads 4

# Don't download results (watch remotely instead)
./scripts/remote/run_tests_remote.sh --no-download
```

### monitor_tests.sh
Watch test execution in real-time.

```bash
# Monitor currently running tests
./scripts/remote/monitor_tests.sh

# Shows:
# - Live test count and progress
# - Elapsed time
# - Failed tests
# - System resource usage
```

## Common Workflows

### Workflow 1: Quick Local Changes → Test

```bash
# Make changes
vim src/main.rs

# Deploy and test
./scripts/remote/deploy_to_maitai.sh
./scripts/remote/run_tests_remote.sh

# Results in: ./test_results/<timestamp>/
```

### Workflow 2: Hardware Integration Testing

```bash
# Deploy code
./scripts/remote/deploy_to_maitai.sh

# Run hardware tests in separate window
./scripts/remote/run_tests_remote.sh --suite hardware

# Monitor in other window
./scripts/remote/monitor_tests.sh

# When done, check results
ls -lah ./test_results/
cat ./test_results/*/test_output.log
```

### Workflow 3: Long-Running Tests with Monitoring

```bash
# Terminal 1: Start tests
./scripts/remote/run_tests_remote.sh --release --timeout 7200

# Terminal 2: Monitor progress
./scripts/remote/monitor_tests.sh

# Terminal 3: Manual SSH if needed
ssh maitai-eos
cd ~/rust-daq
tail -f test.log
```

### Workflow 4: Incremental Code Updates

```bash
# After small change, use incremental sync
rsync -avz src/ maitai-eos:~/rust-daq/src/
rsync -avz Cargo.toml maitai-eos:~/rust-daq/

# Quick test
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib'
```

## SSH Configuration

Ensure `~/.ssh/config` includes:

```ssh-config
Host maitai-eos
    HostName 100.91.139.XX
    User maitai
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 60
    ServerAliveCountMax 3
    Compression yes
```

Replace `100.91.139.XX` with actual Tailscale IP from:
```bash
tailscale status | grep maitai
```

## File Organization

```
docs/testing/
├── README.md                    # This file
├── SSH_ACCESS_GUIDE.md         # SSH setup and troubleshooting
├── REMOTE_TESTING_GUIDE.md     # Testing procedures
├── FILE_TRANSFER_GUIDE.md      # File sync strategies

scripts/remote/
├── deploy_to_maitai.sh         # Deploy code to remote
├── run_tests_remote.sh         # Run tests remotely
├── monitor_tests.sh            # Monitor test progress
└── README.md                    # Script documentation
```

## Test Results Storage

Test results are automatically downloaded to:
```
./test_results/<YYYYMMDD_HHMMSS>/
├── manifest.txt                # Metadata about the run
├── test_output.log            # Full test output
├── results/                    # Test result files
└── *.log                       # Additional logs
```

Each test run gets a timestamped directory to preserve history.

## Performance Tips

### Faster Deployments
- Use incremental sync for source changes: `rsync -avz src/ maitai-eos:~/rust-daq/src/`
- Exclude large directories: `--exclude 'target' --exclude '.git'`
- Compress transfers: built-in to rsync and scp

### Faster Tests
- Run in release mode: `--release` flag
- Run only needed test suite: `--suite lib` or `--suite hardware`
- Limit test threads: `--threads 4`
- Skip unit tests for integration testing: `--suite integration`

### Network Optimization
- See FILE_TRANSFER_GUIDE.md for bandwidth limiting
- Use compression for slow connections
- Use no-compress for fast local networks

## Troubleshooting

### SSH Issues
See **SSH_ACCESS_GUIDE.md** - Section "Step 5: Troubleshooting SSH Issues"

Key points:
- Check Tailscale: `tailscale status`
- Test connection: `ssh -vvv maitai-eos`
- Verify key: `ssh-add -l`

### Test Issues
See **REMOTE_TESTING_GUIDE.md** - Section "Troubleshooting Remote Testing"

Key points:
- Check Rust: `ssh maitai-eos 'rustc --version'`
- Check disk: `ssh maitai-eos 'df -h'`
- Check processes: `ssh maitai-eos 'pgrep cargo'`

### File Transfer Issues
See **FILE_TRANSFER_GUIDE.md** - Section "Troubleshooting File Transfers"

Key points:
- Check permissions: `ssh maitai-eos 'ls -la ~/rust-daq'`
- Check disk space: `ssh maitai-eos 'df -h'`
- Try rsync dry-run: `rsync -avz --dry-run ...`

## SSH Connection Details

- **Host**: maitai-eos (Tailscale address)
- **User**: maitai
- **Port**: 22 (SSH default)
- **VPN**: Tailscale (required)
- **Auth**: SSH key (Ed25519 recommended)

Find actual IP:
```bash
tailscale status | grep maitai-eos
# Output: maitai-eos (100.91.139.XX) linux; idle, tx 1234 rx 5678
```

## Network Requirements

- Tailscale VPN connected
- SSH enabled on maitai-eos
- Port 22 open (standard SSH)
- Network stability for long tests

## Common Issues and Solutions

| Issue | Solution |
|-------|----------|
| SSH: Permission denied | See SSH_ACCESS_GUIDE.md - "Issue: Permission denied (publickey)" |
| SSH: Connection refused | Ensure Tailscale connected: `tailscale status` |
| SSH: Timeout | Increase ServerAliveInterval in ~/.ssh/config |
| Cargo: Command not found | Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Tests hang | Use `--timeout` flag: `run_tests_remote.sh --timeout 300` |
| Out of disk | Run: `ssh maitai-eos 'cd ~/rust-daq && cargo clean'` |
| rsync slow | Add compression or limit threads |

## Advanced Usage

### Custom SSH Command
```bash
# Direct ssh for full control
ssh -t maitai-eos 'cd ~/rust-daq && cargo test --lib'

# Multiple commands
ssh maitai-eos << 'EOF'
cd ~/rust-daq
cargo build --release
cargo test --release
EOF
```

### Persistent Sessions with tmux
```bash
# Create session
ssh maitai-eos 'tmux new-session -d -s tests'

# Run tests in session
ssh maitai-eos 'tmux send-keys -t tests "cd ~/rust-daq && cargo test" Enter'

# Attach interactively
ssh -t maitai-eos 'tmux attach -t tests'
```

### Port Forwarding
```bash
# Forward GUI/web services
ssh -L 8080:localhost:8080 maitai-eos

# Multiple ports
ssh -L 8000:localhost:8000 -L 8080:localhost:8080 maitai-eos
```

## Additional Resources

- SSH Manual: `man ssh`
- rsync Manual: `man rsync`
- scp Manual: `man scp`
- Git Documentation: https://git-scm.com/doc

## Support

For issues:

1. Check relevant documentation section above
2. See "Troubleshooting" section in specific guide
3. Run diagnostic command: `ssh maitai-eos 'uname -a && rustc --version && cargo --version'`
4. Check logs: `cat test_results/*/test_output.log`

## Next Steps

1. Start with SSH_ACCESS_GUIDE.md for initial setup
2. Run deploy_to_maitai.sh to verify everything works
3. Use run_tests_remote.sh for daily testing
4. Refer to FILE_TRANSFER_GUIDE.md for large data transfers
