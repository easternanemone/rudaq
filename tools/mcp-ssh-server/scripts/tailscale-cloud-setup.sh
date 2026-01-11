#!/bin/bash
#
# Tailscale Setup for Claude.ai Cloud Environments
#
# This script is called by SessionStart hook to connect the cloud VM
# to your Tailscale network for secure access to private infrastructure.
#
# Required environment variables (set in Claude.ai cloud environment):
#   TS_AUTHKEY - Tailscale auth key (ephemeral, tagged)
#
# Optional environment variables:
#   TS_HOSTNAME - Custom hostname for this node (default: claude-cloud-$RANDOM)
#   SSH_HOST - Target SSH host on tailnet (for verification)
#

LOG_PREFIX="[tailscale-setup]"

log_info() { echo "$LOG_PREFIX INFO: $1"; }
log_warn() { echo "$LOG_PREFIX WARN: $1"; }
log_error() { echo "$LOG_PREFIX ERROR: $1" >&2; }

# Check if we're in a cloud environment
if [[ -z "$CLAUDE_ENV_FILE" ]]; then
    log_info "Not in cloud environment (CLAUDE_ENV_FILE not set), skipping Tailscale setup"
    exit 0
fi

# Check for auth key
if [[ -z "$TS_AUTHKEY" ]]; then
    log_info "TS_AUTHKEY not set, skipping Tailscale setup"
    exit 0
fi

log_info "Starting Tailscale setup for Claude cloud environment"

# Use a local directory for Tailscale (no root needed)
TS_DIR="${HOME}/.tailscale"
TS_STATE="${TS_DIR}/tailscaled.state"
TS_SOCKET="${TS_DIR}/tailscaled.sock"
mkdir -p "$TS_DIR"

# Check if Tailscale is already installed
if command -v tailscale &> /dev/null; then
    log_info "Tailscale already installed at $(which tailscale)"
    TAILSCALE_BIN="tailscale"
    TAILSCALED_BIN="tailscaled"
else
    log_info "Downloading Tailscale static binary..."

    # Download static binary (doesn't need apt/root)
    TS_VERSION="1.78.3"
    ARCH="amd64"
    TS_TARBALL="tailscale_${TS_VERSION}_${ARCH}.tgz"

    curl -fsSL "https://pkgs.tailscale.com/stable/${TS_TARBALL}" -o "${TS_DIR}/${TS_TARBALL}"
    tar -xzf "${TS_DIR}/${TS_TARBALL}" -C "${TS_DIR}" --strip-components=1
    rm "${TS_DIR}/${TS_TARBALL}"

    TAILSCALE_BIN="${TS_DIR}/tailscale"
    TAILSCALED_BIN="${TS_DIR}/tailscaled"

    chmod +x "$TAILSCALE_BIN" "$TAILSCALED_BIN"
    log_info "Tailscale installed to ${TS_DIR}"
fi

# Generate hostname if not provided
TS_HOSTNAME="${TS_HOSTNAME:-claude-cloud-$(date +%s | tail -c 6)}"

# Start Tailscale daemon in userspace mode (no TUN device, no root)
log_info "Starting tailscaled in userspace mode..."
"$TAILSCALED_BIN" \
    --state="$TS_STATE" \
    --socket="$TS_SOCKET" \
    --tun=userspace-networking \
    --socks5-server=localhost:1055 \
    --outbound-http-proxy-listen=localhost:1056 \
    2>&1 &

TAILSCALED_PID=$!
sleep 3

if ! kill -0 "$TAILSCALED_PID" 2>/dev/null; then
    log_error "tailscaled failed to start"
    exit 1
fi

log_info "tailscaled running (PID: $TAILSCALED_PID)"

# Connect to Tailscale network
log_info "Connecting to Tailscale network as '$TS_HOSTNAME'..."
"$TAILSCALE_BIN" --socket="$TS_SOCKET" up \
    --authkey="$TS_AUTHKEY" \
    --hostname="$TS_HOSTNAME" \
    --accept-routes \
    2>&1

# Wait for connection
log_info "Waiting for Tailscale connection..."
for i in {1..30}; do
    if "$TAILSCALE_BIN" --socket="$TS_SOCKET" status --json 2>/dev/null | grep -q '"BackendState":"Running"'; then
        log_info "Tailscale connected!"
        break
    fi
    sleep 1
done

# Show status
"$TAILSCALE_BIN" --socket="$TS_SOCKET" status

# Export environment for subsequent commands
{
    echo "export TAILSCALE_CONNECTED=1"
    echo "export TS_SOCKET=$TS_SOCKET"
    echo "export PATH=${TS_DIR}:\$PATH"
    # For userspace networking, use the SOCKS5 proxy
    echo "export ALL_PROXY=socks5://localhost:1055"
    echo "export HTTP_PROXY=http://localhost:1056"
    echo "export HTTPS_PROXY=http://localhost:1056"
} >> "$CLAUDE_ENV_FILE"

# Verify SSH connectivity if SSH_HOST is set
if [[ -n "$SSH_HOST" ]]; then
    log_info "Testing connectivity to $SSH_HOST..."
    if "$TAILSCALE_BIN" --socket="$TS_SOCKET" ping "$SSH_HOST" --timeout=5s 2>&1; then
        log_info "Host $SSH_HOST is reachable on tailnet"
        echo "export SSH_HOST=$SSH_HOST" >> "$CLAUDE_ENV_FILE"
        [[ -n "$SSH_USER" ]] && echo "export SSH_USER=$SSH_USER" >> "$CLAUDE_ENV_FILE"
    else
        log_warn "Host $SSH_HOST not reachable yet (may need a moment)"
    fi
fi

log_info "Tailscale setup complete"
log_info "Use SOCKS5 proxy (localhost:1055) or HTTP proxy (localhost:1056) for tailnet access"

# Create ssh wrapper using tailscale ssh (since ssh client may not be installed)
SSH_WRAPPER="${TS_DIR}/ssh"
cat > "$SSH_WRAPPER" << 'WRAPPER_EOF'
#!/bin/bash
# SSH wrapper using tailscale ssh
TS_DIR="${HOME}/.tailscale"
TS_SOCKET="${TS_DIR}/tailscaled.sock"
TAILSCALE_BIN="${TS_DIR}/tailscale"

# If tailscale is in PATH, use it directly
if command -v tailscale &> /dev/null; then
    TAILSCALE_BIN="tailscale"
fi

# Parse arguments - tailscale ssh uses user@host format
exec "$TAILSCALE_BIN" --socket="$TS_SOCKET" ssh "$@"
WRAPPER_EOF
chmod +x "$SSH_WRAPPER"

log_info "SSH wrapper created at $SSH_WRAPPER"
log_info "Use: $SSH_WRAPPER user@host (or add ${TS_DIR} to PATH)"
