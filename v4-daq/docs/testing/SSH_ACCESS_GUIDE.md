# SSH Access Guide for maitai-eos Hardware Testing

This guide provides complete instructions for establishing SSH connections to the maitai-eos hardware testing system via Tailscale VPN.

## Prerequisites

Before attempting SSH connection, ensure you have:

1. SSH key pair (Ed25519 recommended)
2. Tailscale installed and configured on your local machine
3. Access to the maitai-eos system on the Tailscale network
4. Git installed (for deploying code)
5. rsync installed (for file transfer)

## Step 1: SSH Key Setup

### Generate SSH Key (if needed)

```bash
# Generate Ed25519 key (recommended for better security and performance)
ssh-keygen -t ed25519 -C "your-email@example.com" -f ~/.ssh/id_ed25519

# Or use RSA if Ed25519 is not available
ssh-keygen -t rsa -b 4096 -C "your-email@example.com" -f ~/.ssh/id_rsa
```

### Add Key to SSH Agent

```bash
# Start SSH agent (usually auto-running on macOS/Linux)
eval "$(ssh-agent -s)"

# Add your private key
ssh-add ~/.ssh/id_ed25519
# or
ssh-add ~/.ssh/id_rsa

# Verify key was added
ssh-add -l
```

### Distribute Public Key to maitai-eos

Contact the system administrator to add your public key (`~/.ssh/id_ed25519.pub` or `~/.ssh/id_rsa.pub`) to `/home/maitai/.ssh/authorized_keys` on maitai-eos.

If you have access to maitai-eos and `ssh-copy-id` is available:

```bash
# First-time setup (you'll be prompted for password)
ssh-copy-id -i ~/.ssh/id_ed25519.pub maitai@<tailscale-ip>
```

## Step 2: Tailscale VPN Configuration

### Verify Tailscale Installation

```bash
# Check if Tailscale is installed
tailscale version

# If not installed, download from https://tailscale.com/download
# macOS: brew install tailscale
# Linux: sudo apt-get install tailscale (Debian/Ubuntu)
```

### Connect to Tailscale Network

```bash
# Start Tailscale (if not already running)
sudo tailscale up

# Check your Tailscale IP
tailscale ip -4

# List all available nodes on the network
tailscale status
```

### Find maitai-eos IP Address

```bash
# Look for maitai-eos in the Tailscale status output
tailscale status | grep -i maitai

# Example output:
# maitai-eos (100.91.139.XX) linux; idle, tx 1234 rx 5678
```

## Step 3: SSH Configuration File Setup

Create or update `~/.ssh/config` with the following entry:

```ssh-config
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

    # Security
    StrictHostKeyChecking accept-new
    UserKnownHostsFile ~/.ssh/known_hosts

    # Port forwarding (for GUI/monitoring)
    LocalForward 8000 localhost:8000
    LocalForward 8080 localhost:8080
    LocalForward 9090 localhost:9090
```

Replace `100.91.139.XX` with the actual Tailscale IP found in step 2.

### Verify SSH Config

```bash
# Check syntax
ssh -G maitai-eos | head -20

# Should show configuration without errors
```

## Step 4: Initial SSH Connection Test

### Connect to maitai-eos

```bash
# First connection (may prompt to add to known_hosts)
ssh maitai-eos

# You should see a login banner and shell prompt
```

### Verify Connection

Once connected, run:

```bash
# Check system info
uname -a
whoami
pwd

# Verify rust toolchain
rustc --version
cargo --version

# Check Tailscale connectivity
ip addr show | grep -i tailscale
```

### Disconnect

```bash
exit
```

## Step 5: Troubleshooting SSH Issues

### Issue: "Connection refused"

**Cause**: SSH server not running or port not open.

**Solution**:
```bash
# Check if maitai-eos is on Tailscale
tailscale status | grep maitai

# Try pinging first
ping 100.91.139.XX -c 3

# Verify SSH is running (if you have access)
ssh maitai-eos 'sudo systemctl status ssh'
```

### Issue: "Permission denied (publickey)"

**Cause**: Public key not in authorized_keys on remote system.

**Solution**:
```bash
# Copy public key again (using password auth)
ssh-copy-id -i ~/.ssh/id_ed25519.pub maitai@100.91.139.XX

# Or ask admin to add your key manually to /home/maitai/.ssh/authorized_keys
```

### Issue: "Timeout" or "Connection reset by peer"

**Cause**: Network connectivity issue or firewall blocking.

**Solution**:
```bash
# Ensure Tailscale is connected
tailscale status

# Check your Tailscale IP
tailscale ip -4

# Try verbose SSH to diagnose
ssh -vvv maitai-eos

# Reconnect Tailscale if needed
sudo tailscale up
```

### Issue: "Host key verification failed"

**Cause**: SSH fingerprint mismatch (security concern).

**Solution**:
```bash
# If you trust the system, remove old key
ssh-keygen -R 100.91.139.XX

# Try connecting again (will ask to accept new key)
ssh maitai-eos

# Type 'yes' to accept the new fingerprint
```

### Issue: "SSH key not found"

**Cause**: SSH agent not running or key not added.

**Solution**:
```bash
# Start SSH agent
eval "$(ssh-agent -s)"

# Add key
ssh-add ~/.ssh/id_ed25519

# Verify
ssh-add -l
```

## Port Forwarding for Remote Development

### GUI Access (if needed)

```bash
# Forward X11 (graphical display)
ssh -Y maitai-eos

# Or in ~/.ssh/config, add:
# ForwardX11 yes
# ForwardX11Trusted yes
```

### Web Service Access

```bash
# Forward port 8080 from remote to local
ssh -L 8080:localhost:8080 maitai-eos

# Then access locally: http://localhost:8080
```

### Database/API Access

```bash
# Forward any remote service port
ssh -L 9000:localhost:9000 maitai-eos

# Multiple ports
ssh -L 8000:localhost:8000 -L 8080:localhost:8080 -L 9090:localhost:9090 maitai-eos
```

## Security Best Practices

### Key Security

1. **Protect Your Private Key**
   ```bash
   # Ensure correct permissions
   chmod 600 ~/.ssh/id_ed25519
   chmod 600 ~/.ssh/id_rsa
   chmod 700 ~/.ssh
   ```

2. **Keep Keys Secure**
   - Never share private keys
   - Never commit keys to version control
   - Use SSH passphrase for extra protection
   - Consider hardware security keys for critical systems

### Connection Security

1. **Use SSH Agent Forwarding Carefully**
   ```bash
   # Only enable when needed
   # In ~/.ssh/config: ForwardAgent yes
   ```

2. **Set Connection Timeouts**
   ```bash
   # Already in config example:
   ServerAliveInterval 60
   ServerAliveCountMax 3
   ```

3. **Disable Unused Features**
   ```bash
   # In ~/.ssh/config
   AllowAgentForwarding no   # if not needed
   AllowTcpForwarding no     # if not needed
   ```

## Advanced SSH Features

### Using SSH Tunnels for Proxying

```bash
# Create a SOCKS proxy through maitai-eos
ssh -D 1080 maitai-eos

# Then configure applications to use 127.0.0.1:1080 as SOCKS proxy
```

### SSH Key Rotation

```bash
# Generate new key
ssh-keygen -t ed25519 -C "rotated-key" -f ~/.ssh/id_ed25519_new

# Copy to remote
ssh-copy-id -i ~/.ssh/id_ed25519_new.pub maitai@100.91.139.XX

# Update ~/.ssh/config to use new key
# IdentityFile ~/.ssh/id_ed25519_new

# Verify new key works
ssh maitai-eos 'echo "Connected with new key"'

# Remove old key from remote once verified
# Then delete local old key: rm ~/.ssh/id_ed25519
```

### Batch SSH Commands

```bash
# Run single command and disconnect
ssh maitai-eos 'cargo --version && rustc --version'

# Run script on remote
ssh maitai-eos 'bash /home/maitai/scripts/setup.sh'

# Multiple commands
ssh maitai-eos << 'EOF'
cd ~/rust-daq
git status
cargo test --lib
EOF
```

## Session Persistence

### Using tmux for Persistent Sessions

```bash
# Create persistent session on remote
ssh maitai-eos 'tmux new-session -d -s build'

# Run command in session
ssh maitai-eos 'tmux send-keys -t build:0 "cd ~/rust-daq && cargo build" Enter'

# Attach to session (interactive)
ssh maitai-eos -t 'tmux attach -t build'

# View session status
ssh maitai-eos 'tmux list-sessions'
```

## Automated Connection Testing

Create a script to test SSH connectivity:

```bash
#!/bin/bash
# File: scripts/local/test_ssh_access.sh

REMOTE_HOST="maitai-eos"
CHECKS_PASSED=0
CHECKS_TOTAL=0

echo "Testing SSH access to $REMOTE_HOST..."

# Test 1: Tailscale connectivity
echo -n "1. Checking Tailscale... "
CHECKS_TOTAL=$((CHECKS_TOTAL + 1))
if tailscale status > /dev/null 2>&1; then
    echo "OK"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo "FAILED - Tailscale not connected"
fi

# Test 2: SSH key availability
echo -n "2. Checking SSH keys... "
CHECKS_TOTAL=$((CHECKS_TOTAL + 1))
if ssh-add -l > /dev/null 2>&1; then
    echo "OK"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo "WARNING - SSH agent not running"
fi

# Test 3: SSH connection
echo -n "3. Testing SSH connection... "
CHECKS_TOTAL=$((CHECKS_TOTAL + 1))
if ssh -o ConnectTimeout=5 "$REMOTE_HOST" 'echo "Connected"' > /dev/null 2>&1; then
    echo "OK"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo "FAILED"
fi

# Test 4: Remote toolchain
echo -n "4. Checking Rust toolchain... "
CHECKS_TOTAL=$((CHECKS_TOTAL + 1))
if ssh "$REMOTE_HOST" 'rustc --version && cargo --version' > /dev/null 2>&1; then
    echo "OK"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo "FAILED"
fi

# Summary
echo ""
echo "Results: $CHECKS_PASSED/$CHECKS_TOTAL checks passed"

if [ $CHECKS_PASSED -eq $CHECKS_TOTAL ]; then
    echo "All checks passed! Ready for remote testing."
    exit 0
else
    echo "Some checks failed. See troubleshooting section above."
    exit 1
fi
```

## Quick Reference

```bash
# Connect to maitai-eos
ssh maitai-eos

# Run command remotely
ssh maitai-eos 'cargo test'

# Copy file to remote
scp local_file.txt maitai-eos:/home/maitai/

# Copy file from remote
scp maitai-eos:/home/maitai/remote_file.txt .

# Sync directory (see FILE_TRANSFER_GUIDE.md for details)
rsync -avz --delete src/ maitai-eos:~/rust-daq/src/

# Forward ports
ssh -L 8080:localhost:8080 maitai-eos
```

## Next Steps

1. Follow Step 1-4 above to establish initial SSH connection
2. Run troubleshooting tests if any issues occur
3. See `REMOTE_TESTING_GUIDE.md` for deploying and running tests
4. See `FILE_TRANSFER_GUIDE.md` for efficient file transfers
5. Use automation scripts in `scripts/remote/` for common tasks
