# File Transfer Guide for maitai-eos

Efficient file transfer between your local machine and maitai-eos using rsync, scp, and git.

## Quick Reference

```bash
# Copy single file to remote
scp local_file.txt maitai-eos:/home/maitai/

# Copy single file from remote
scp maitai-eos:/home/maitai/remote_file.txt .

# Sync source code (fast, incremental)
rsync -avz --delete src/ maitai-eos:~/rust-daq/src/

# Sync with compression and bandwidth limit
rsync -avz --bwlimit=10000 --compress-level=6 ./ maitai-eos:~/rust-daq/

# Copy results back with timestamp
scp -r maitai-eos:~/rust-daq/results/ ./results_$(date +%Y%m%d)
```

## Method 1: Using SCP (Simple Copy)

SCP is simple but less efficient for large directories. Use for:
- Single files
- Small number of files
- One-time copies

### Copy to Remote

```bash
# Single file
scp local_file.txt maitai-eos:/home/maitai/

# Multiple files
scp file1.txt file2.rs file3.md maitai-eos:/home/maitai/

# Entire directory
scp -r local_directory/ maitai-eos:/home/maitai/

# With compression (slower locally, faster over network)
scp -C local_file.txt maitai-eos:/home/maitai/

# Specify source path with wildcards
scp 'src/*.rs' maitai-eos:~/rust-daq/src/
```

### Copy from Remote

```bash
# Single file
scp maitai-eos:/home/maitai/remote_file.txt .

# Entire directory
scp -r maitai-eos:/home/maitai/results/ ./results/

# Multiple files
scp 'maitai-eos:/home/maitai/*.log' ./logs/
```

### SCP with Custom Port/Identity

```bash
# Using specific SSH key
scp -i ~/.ssh/id_ed25519 local_file.txt maitai-eos:/home/maitai/

# Custom SSH options
scp -o "ServerAliveInterval=30" file.txt maitai-eos:/home/maitai/
```

## Method 2: Using rsync (Recommended for Code)

rsync is efficient, supports incremental updates, and is perfect for syncing source code. Use for:
- Syncing entire projects
- Incremental updates
- Excluding files/directories
- Bandwidth-limited transfers

### Basic rsync Syntax

```bash
# Sync source to remote
rsync -avz source/ maitai-eos:~/rust-daq/source/

# Sync from remote to local
rsync -avz maitai-eos:~/rust-daq/results/ ./results/

# Bidirectional sync (careful - can delete!)
rsync -avz --delete local/ maitai-eos:~/rust-daq/local/
```

### Understanding rsync Flags

- `-a` : Archive mode (preserves permissions, timestamps, etc.)
- `-v` : Verbose (shows files being transferred)
- `-z` : Compress during transfer
- `--delete` : Delete files on destination not in source
- `--exclude` : Skip matching files/directories
- `--include` : Only sync matching files
- `--bwlimit` : Limit bandwidth (KB/s)
- `--progress` : Show progress for large files
- `--partial` : Keep partially transferred files
- `-S` : Handle sparse files efficiently

### Exclude Common Build Artifacts

```bash
# Don't sync target/ and other common build files
rsync -avz --delete \
  --exclude 'target' \
  --exclude '.git' \
  --exclude 'Cargo.lock' \
  --exclude '.DS_Store' \
  --exclude '*.swp' \
  --exclude '.idea' \
  ./ maitai-eos:~/rust-daq/
```

### Incremental Sync (After Initial Deployment)

```bash
# Fast update: only source files
rsync -avz src/ maitai-eos:~/rust-daq/src/

# Also update Cargo files
rsync -avz Cargo.toml Cargo.lock maitai-eos:~/rust-daq/

# Or sync everything except target
rsync -avz --delete \
  --exclude 'target' \
  --exclude '.git' \
  ./ maitai-eos:~/rust-daq/
```

### Bandwidth-Limited Transfer

```bash
# Limit to 10 MB/s (10000 KB/s)
rsync -avz --bwlimit=10000 ./ maitai-eos:~/rust-daq/

# Useful for:
# - Shared network connections
# - Avoiding network saturation
# - Keeping other services responsive
```

### Show What Would Be Transferred (Dry Run)

```bash
# See what rsync would do without actually doing it
rsync -avz --dry-run ./ maitai-eos:~/rust-daq/

# Useful to verify before syncing large amounts
```

### Transfer with Progress

```bash
# Show progress for each file and overall
rsync -avz --progress src/ maitai-eos:~/rust-daq/src/

# For large files
rsync -avz --progress --bwlimit=50000 ./ maitai-eos:~/rust-daq/
```

### Common rsync Workflows

**Initial Deployment:**
```bash
rsync -avz --delete \
  --exclude 'target' \
  --exclude '.git' \
  --exclude 'Cargo.lock' \
  ./ maitai-eos:~/rust-daq/
```

**Update After Code Changes:**
```bash
# Fast sync just source files
rsync -avz src/ maitai-eos:~/rust-daq/src/
rsync -avz Cargo.toml maitai-eos:~/rust-daq/
```

**Download Results:**
```bash
# Pull test results back
rsync -avz maitai-eos:~/rust-daq/results/ ./results/
rsync -avz maitai-eos:~/rust-daq/*.log ./logs/
```

**Selective Sync:**
```bash
# Only sync Rust files and configs
rsync -avz --include '*.rs' --include 'Cargo.*' --include '*.toml' \
  --exclude '*' src/ maitai-eos:~/rust-daq/src/
```

**With Partial Transfer Resumption:**
```bash
# Useful for unreliable connections
rsync -avz --partial --progress \
  large_file.tar.gz maitai-eos:/home/maitai/
```

## Method 3: Using Git

If both systems have git history, syncing via git is efficient.

### Setup (One Time)

```bash
# On remote, clone the repository
ssh maitai-eos 'git clone /path/to/origin/repo ~/rust-daq'

# Or if origin is on GitHub
ssh maitai-eos 'cd ~/rust-daq && git remote set-url origin https://github.com/your/repo.git'
```

### Update Code via Git

```bash
# Pull latest changes
ssh maitai-eos 'cd ~/rust-daq && git fetch origin && git reset --hard origin/main'

# Checkout specific branch
ssh maitai-eos 'cd ~/rust-daq && git fetch origin && git checkout feature-branch'

# Or via SSH
ssh -t maitai-eos << 'EOF'
cd ~/rust-daq
git fetch origin
git reset --hard origin/main
git log --oneline -5
EOF
```

### Advantages and Disadvantages

**Advantages:**
- Very fast for code updates (only changes transferred)
- Clean history with git
- No conflicts with ignore files
- Works across networks

**Disadvantages:**
- Requires git repository on remote
- Syncs entire commit history
- May include large binary files if not in .gitignore

## Method 4: Using tar for Batching

For one-time large transfers, tar + compression can be efficient.

### Create Compressed Archive on Remote

```bash
# Compress on remote, download once
ssh maitai-eos 'cd ~/rust-daq && tar czf results.tar.gz results/ logs/ *.log'

# Download archive
scp maitai-eos:~/rust-daq/results.tar.gz ./

# Extract locally
tar xzf results.tar.gz
```

### Send Multiple Files in One Package

```bash
# Local: create archive
tar czf upload.tar.gz config/ examples/ scripts/

# Send to remote
scp upload.tar.gz maitai-eos:/home/maitai/

# Remote: extract and verify
ssh maitai-eos 'cd /home/maitai && tar xzf upload.tar.gz && ls'
```

## Automating File Transfers

### Automated Sync Script

Create a reusable script for deployments:

```bash
#!/bin/bash
# scripts/remote/sync_to_remote.sh

REMOTE_HOST="maitai-eos"
REMOTE_DIR="~/rust-daq"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}Syncing to $REMOTE_HOST:$REMOTE_DIR${NC}"

# Verify SSH connection
if ! ssh -o ConnectTimeout=5 "$REMOTE_HOST" 'echo "Connected"' > /dev/null 2>&1; then
    echo -e "${RED}Failed to connect to $REMOTE_HOST${NC}"
    exit 1
fi

# Create remote directory
ssh "$REMOTE_HOST" "mkdir -p $REMOTE_DIR"

# Perform sync with rsync
rsync -avz --delete \
  --exclude 'target' \
  --exclude '.git' \
  --exclude 'Cargo.lock' \
  --exclude '*.swp' \
  --exclude '.DS_Store' \
  ./ "${REMOTE_HOST}:${REMOTE_DIR}/"

if [ $? -eq 0 ]; then
    echo -e "${GREEN}Sync completed successfully${NC}"
else
    echo -e "${RED}Sync failed${NC}"
    exit 1
fi

# Verify remote
echo -e "${YELLOW}Verifying remote...${NC}"
ssh "$REMOTE_HOST" "cd $REMOTE_DIR && ls -la && cargo check"
```

### Automated Results Download

```bash
#!/bin/bash
# scripts/local/download_results.sh

REMOTE_HOST="maitai-eos"
REMOTE_DIR="~/rust-daq"
LOCAL_DIR="./results"

mkdir -p "$LOCAL_DIR"

echo "Downloading test results from $REMOTE_HOST..."

# Download with timestamp
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_DIR="$LOCAL_DIR/run_$TIMESTAMP"

mkdir -p "$RESULTS_DIR"

# Download logs and results
rsync -avz "$REMOTE_HOST:${REMOTE_DIR}/results/" "$RESULTS_DIR/results/"
rsync -avz "$REMOTE_HOST:${REMOTE_DIR}/*.log" "$RESULTS_DIR/" 2>/dev/null || true

# Create manifest
cat > "$RESULTS_DIR/manifest.txt" << EOF
Downloaded: $(date)
From: $REMOTE_HOST
Tests: $(ls "$RESULTS_DIR"/results 2>/dev/null | wc -l) files
EOF

echo "Results saved to: $RESULTS_DIR"
ls -lah "$RESULTS_DIR"
```

### Continuous Sync Daemon

```bash
#!/bin/bash
# scripts/remote/sync_daemon.sh
# Run in background to keep remote in sync

REMOTE_HOST="maitai-eos"
SYNC_INTERVAL=60

echo "Starting sync daemon (interval: ${SYNC_INTERVAL}s)"

while true; do
    # Sync source files
    rsync -az --delete \
      --exclude 'target' \
      --exclude '.git' \
      src/ "${REMOTE_HOST}:~/rust-daq/src/" 2>/dev/null

    # Sync Cargo files if changed
    rsync -az Cargo.toml Cargo.lock "${REMOTE_HOST}:~/rust-daq/" 2>/dev/null || true

    sleep "$SYNC_INTERVAL"
done
```

Run in background:
```bash
./scripts/remote/sync_daemon.sh &
```

## Network Optimization

### Compression Levels

```bash
# Default compression (6)
rsync -avz src/ maitai-eos:~/rust-daq/src/

# High compression (better for slow networks)
rsync -avz --compress-level=9 src/ maitai-eos:~/rust-daq/src/

# Low compression (faster for fast networks)
rsync -avz --compress-level=1 src/ maitai-eos:~/rust-daq/src/

# No compression (fastest for LAN)
rsync -av --no-compress src/ maitai-eos:~/rust-daq/src/
```

### Bandwidth Limiting

```bash
# 5 MB/s (5000 KB/s)
rsync -avz --bwlimit=5000 ./ maitai-eos:~/rust-daq/

# Check speed: actual rate is usually 80% of limit
# For 5000 KB/s limit: expect ~4 MB/s
```

### Connection Timeout

```bash
# Increase timeout for slow connections
rsync -avz --timeout=300 ./ maitai-eos:~/rust-daq/
```

## Troubleshooting File Transfers

### Issue: "Permission denied" on remote directory

```bash
# Check remote permissions
ssh maitai-eos 'ls -la ~/rust-daq'

# Fix if needed
ssh maitai-eos 'chmod -R u+rwx ~/rust-daq'

# Or recreate
ssh maitai-eos 'rm -rf ~/rust-daq && mkdir ~/rust-daq'
```

### Issue: "No such file or directory"

```bash
# Verify remote directory exists
ssh maitai-eos 'test -d ~/rust-daq && echo "exists" || echo "missing"'

# Create if missing
ssh maitai-eos 'mkdir -p ~/rust-daq'

# Try sync again
rsync -avz ./ maitai-eos:~/rust-daq/
```

### Issue: Transfer stuck or very slow

```bash
# Check network connectivity
ping -c 5 100.91.139.XX

# Test SSH directly
ssh -o ConnectTimeout=5 maitai-eos 'date'

# Reduce compression if CPU-bound
rsync -av --no-compress ./ maitai-eos:~/rust-daq/

# Or increase bandwidth limit if network-bound
rsync -avz --bwlimit=50000 ./ maitai-eos:~/rust-daq/
```

### Issue: "rsync: write failed: No space left on device"

```bash
# Check remote disk
ssh maitai-eos 'df -h'

# Clean up remote
ssh maitai-eos 'cd ~/rust-daq && cargo clean && rm -rf target/'

# Or remove old results
ssh maitai-eos 'rm -rf ~/rust-daq/results/*'
```

## Performance Comparison

For syncing entire project (excluding target/):

| Method | Initial | Updates | Speed | Best For |
|--------|---------|---------|-------|----------|
| SCP | Slow | N/A | Varies | Small files |
| rsync | Medium | Very Fast | ~20-50 MB/s | Code sync |
| Git | Medium | Very Fast | ~50+ MB/s | Large repos |
| tar | Fast | N/A | ~100+ MB/s | One-time bulk |

Typical times for rust-daq (~50 MB source):
- SCP: 5-10 seconds
- rsync (initial): 3-5 seconds
- rsync (update): 0.5-1 second
- Git: 2-3 seconds
- tar: 0.5-1 second

## Best Practices

1. **Use rsync for regular syncing** - Efficient and incremental
2. **Exclude build artifacts** - Use `--exclude 'target'`
3. **Use compression** - Usually worth the CPU cost
4. **Limit bandwidth on shared networks** - Use `--bwlimit`
5. **Verify transfers** - Always check sizes after transfer
6. **Automate repetitive tasks** - Use scripts
7. **Monitor disk space** - Check remote disk before large transfers

## Next Steps

1. See `SSH_ACCESS_GUIDE.md` for SSH setup
2. See `REMOTE_TESTING_GUIDE.md` for testing procedures
3. Use the provided automation scripts in `scripts/remote/`
4. Customize sync scripts for your workflow
