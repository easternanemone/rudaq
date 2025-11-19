# SSH & Remote Testing Quick Reference Card

## SSH Connection (One-Time Setup)

```bash
# 1. Generate SSH key (if needed)
ssh-keygen -t ed25519 -C "your-email@example.com" -f ~/.ssh/id_ed25519

# 2. Add to SSH agent
eval "$(ssh-agent -s)"
ssh-add ~/.ssh/id_ed25519

# 3. Get Tailscale IP
tailscale status | grep maitai

# 4. Add to ~/.ssh/config
# Host maitai-eos
#     HostName 100.91.139.XX
#     User maitai
#     IdentityFile ~/.ssh/id_ed25519
#     ServerAliveInterval 60
#     ServerAliveCountMax 3

# 5. Test connection
ssh maitai-eos
```

## Deploy Code

```bash
# Recommended: Use deployment script
./scripts/remote/deploy_to_maitai.sh

# Or manual sync
rsync -avz --delete \
  --exclude 'target' --exclude '.git' \
  ./ maitai-eos:~/rust-daq/
```

## Run Tests

```bash
# Run all tests
./scripts/remote/run_tests_remote.sh

# Library tests only
./scripts/remote/run_tests_remote.sh --suite lib

# Integration tests
./scripts/remote/run_tests_remote.sh --suite integration

# Hardware tests
./scripts/remote/run_tests_remote.sh --suite hardware

# Release mode (faster execution)
./scripts/remote/run_tests_remote.sh --release

# Single-threaded (for debugging)
./scripts/remote/run_tests_remote.sh --threads 1

# Long timeout (in seconds)
./scripts/remote/run_tests_remote.sh --timeout 7200
```

## Monitor Tests

```bash
# Watch real-time progress
./scripts/remote/monitor_tests.sh

# Or manual SSH monitoring
ssh maitai-eos 'tail -f ~/rust-daq/test_output.log'

# Watch resources
ssh maitai-eos 'watch -n 5 "ps aux | grep cargo"'
```

## File Transfer

```bash
# Deploy code (efficient sync)
rsync -avz --delete src/ Cargo.toml maitai-eos:~/rust-daq/

# Download results
scp -r maitai-eos:~/rust-daq/results/ ./

# Download logs
scp maitai-eos:~/rust-daq/*.log ./

# Sync large directories with compression
rsync -avz --compress-level=6 ./ maitai-eos:~/rust-daq/

# Limit bandwidth (e.g., 10 MB/s)
rsync -avz --bwlimit=10000 ./ maitai-eos:~/rust-daq/

# Dry run (see what would be synced)
rsync -avz --dry-run ./ maitai-eos:~/rust-daq/
```

## Remote Commands

```bash
# Build
ssh maitai-eos 'cd ~/rust-daq && cargo build'
ssh maitai-eos 'cd ~/rust-daq && cargo build --release'

# Check (no compile)
ssh maitai-eos 'cd ~/rust-daq && cargo check'

# Quick test
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib'

# Full test with output
ssh maitai-eos 'cd ~/rust-daq && cargo test -- --nocapture'

# Run single test
ssh maitai-eos 'cd ~/rust-daq && cargo test measurement::test_conversion'

# Check Rust
ssh maitai-eos 'rustc --version && cargo --version'

# Check disk
ssh maitai-eos 'df -h'

# Clean build
ssh maitai-eos 'cd ~/rust-daq && cargo clean && cargo build'
```

## Persistent Sessions (tmux)

```bash
# Create session
ssh maitai-eos 'tmux new-session -d -s tests'

# Run in session
ssh maitai-eos 'tmux send-keys -t tests "cd ~/rust-daq && cargo test" Enter'

# Attach (interactive)
ssh -t maitai-eos 'tmux attach -t tests'

# View output without attaching
ssh maitai-eos 'tmux capture-pane -t tests -p'

# Kill session
ssh maitai-eos 'tmux kill-session -t tests'
```

## Troubleshooting

```bash
# Check SSH connection
ssh -vvv maitai-eos 'echo OK'

# Check Tailscale
tailscale status
tailscale ip -4

# Verify SSH key
ssh-add -l

# Test network
ping -c 3 100.91.139.XX

# Check remote Rust
ssh maitai-eos 'which rustc && which cargo'

# Kill hung tests
ssh maitai-eos 'pkill -f cargo'

# Clean disk space
ssh maitai-eos 'cd ~/rust-daq && cargo clean'

# Remove old results
ssh maitai-eos 'rm -rf ~/rust-daq/results/*'

# Full diagnostics
ssh maitai-eos << 'EOF'
echo "=== System Info ==="
uname -a
echo "=== Rust Info ==="
rustc --version && cargo --version
echo "=== Disk ==="
df -h
echo "=== Memory ==="
free -h
echo "=== Network ==="
ip addr show
EOF
```

## Common Workflows

### Quick Test
```bash
./scripts/remote/deploy_to_maitai.sh && \
./scripts/remote/run_tests_remote.sh --suite lib
```

### Full Release Testing
```bash
./scripts/remote/deploy_to_maitai.sh && \
./scripts/remote/run_tests_remote.sh --release --timeout 3600
```

### Hardware Testing
```bash
./scripts/remote/deploy_to_maitai.sh && \
./scripts/remote/run_tests_remote.sh --suite hardware &
./scripts/remote/monitor_tests.sh
```

### Incremental Update + Test
```bash
rsync -avz src/ maitai-eos:~/rust-daq/src/ && \
ssh maitai-eos 'cd ~/rust-daq && cargo test --lib'
```

## Test Results Location

Tests save to: `./test_results/<YYYYMMDD_HHMMSS>/`

```bash
# View latest results
ls -lah ./test_results/
cat ./test_results/*/manifest.txt
cat ./test_results/*/test_output.log
```

## Environment

- **Tailscale IP**: 100.91.139.XX (find with `tailscale status`)
- **SSH User**: maitai
- **SSH Port**: 22
- **Remote Home**: /home/maitai
- **Project Dir**: ~/rust-daq

## Documentation

| Document | Purpose |
|----------|---------|
| SSH_ACCESS_GUIDE.md | Initial SSH setup, troubleshooting |
| REMOTE_TESTING_GUIDE.md | Testing procedures and workflows |
| FILE_TRANSFER_GUIDE.md | Efficient file sync strategies |
| README.md | Overview and structure |

## Key Points to Remember

1. Always use Tailscale VPN
2. Deploy code before testing
3. Use `--release` for faster test execution
4. Monitor progress with `monitor_tests.sh`
5. Results saved with timestamp in `./test_results/`
6. Check logs for failures: `cat test_results/*/test_output.log`
7. Clean disk if needed: `ssh maitai-eos 'cd ~/rust-daq && cargo clean'`

## One-Liner Test Commands

```bash
# Deploy + test
./scripts/remote/deploy_to_maitai.sh && ./scripts/remote/run_tests_remote.sh

# Deploy + test + show results
./scripts/remote/deploy_to_maitai.sh && ./scripts/remote/run_tests_remote.sh && cat test_results/*/test_output.log

# Just run tests (assumes already deployed)
./scripts/remote/run_tests_remote.sh --suite lib

# Background test + monitor in separate terminal
./scripts/remote/run_tests_remote.sh &
sleep 2 && ./scripts/remote/monitor_tests.sh
```

## Emergency Commands

```bash
# Kill all cargo processes
ssh maitai-eos 'pkill -f cargo'

# Reset project directory
ssh maitai-eos 'rm -rf ~/rust-daq && mkdir ~/rust-daq'

# Full cleanup and rebuild
ssh maitai-eos << 'EOF'
cd ~/rust-daq
cargo clean
git reset --hard HEAD
cargo build --release
EOF

# Check system health
ssh maitai-eos << 'EOF'
df -h | grep -E "^\/"
free -h
ps aux | grep -E "^maitai" | head -10
EOF
```

## SSH Config Template

Save as `~/.ssh/config`:

```
Host maitai-eos
    HostName 100.91.139.XX
    User maitai
    IdentityFile ~/.ssh/id_ed25519

    # Connection stability
    ServerAliveInterval 60
    ServerAliveCountMax 3

    # Performance
    Compression yes
    CompressionLevel 6

    # Port forwarding (optional)
    LocalForward 8000 localhost:8000
    LocalForward 8080 localhost:8080
```

Replace `100.91.139.XX` with actual Tailscale IP.
