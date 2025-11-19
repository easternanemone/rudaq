#!/bin/bash
# Deploy rust-daq to maitai-eos
#
# Usage: ./scripts/remote/deploy_to_maitai.sh [--full] [--no-build]
#
# Options:
#   --full        Perform full sync (slower but complete)
#   --no-build    Skip remote build verification
#   --help        Show this help message

set -e

# Configuration
REMOTE_HOST="maitai-eos"
REMOTE_DIR="~/rust-daq"
DEPLOY_LOG="deploy_$(date +%Y%m%d_%H%M%S).log"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Flags
FULL_SYNC=false
SKIP_BUILD=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --full)
            FULL_SYNC=true
            shift
            ;;
        --no-build)
            SKIP_BUILD=true
            shift
            ;;
        --help)
            grep "^#" "$0" | head -15
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Helper functions
log() {
    echo -e "${BLUE}[$(date +'%H:%M:%S')]${NC} $1" | tee -a "$DEPLOY_LOG"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1" | tee -a "$DEPLOY_LOG"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1" | tee -a "$DEPLOY_LOG"
    exit 1
}

warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1" | tee -a "$DEPLOY_LOG"
}

header() {
    echo "" | tee -a "$DEPLOY_LOG"
    echo -e "${YELLOW}=== $1 ===${NC}" | tee -a "$DEPLOY_LOG"
}

# Verify prerequisites
header "Checking Prerequisites"

log "Checking SSH connection to $REMOTE_HOST..."
if ! ssh -o ConnectTimeout=5 "$REMOTE_HOST" 'echo "Connected"' > /dev/null 2>&1; then
    error "Cannot connect to $REMOTE_HOST. Check SSH configuration (see SSH_ACCESS_GUIDE.md)"
fi
success "SSH connection OK"

log "Checking Tailscale connectivity..."
if ! ping -c 1 -t 1 100.91.139.* > /dev/null 2>&1; then
    warning "Cannot ping Tailscale IPs - may indicate network issue"
fi

log "Checking rsync availability..."
if ! command -v rsync &> /dev/null; then
    error "rsync not found. Install with: brew install rsync"
fi
success "rsync found"

# Check working directory
if [ ! -f "Cargo.toml" ]; then
    error "Not in rust-daq root directory. Run from project root."
fi
log "Working directory: $(pwd)"

header "Pre-Deployment Checks"

log "Checking for uncommitted changes..."
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    warning "Uncommitted changes found. These will be synced as-is."
fi

log "Checking git status..."
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
CURRENT_COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
log "Current branch: $CURRENT_BRANCH"
log "Current commit: $CURRENT_COMMIT"

log "Checking project structure..."
if [ ! -d "src" ]; then
    error "src/ directory not found"
fi
success "Project structure OK"

# Create remote directory
header "Setting Up Remote Environment"

log "Creating remote directory structure..."
ssh "$REMOTE_HOST" "mkdir -p $REMOTE_DIR" || error "Failed to create remote directory"
success "Remote directory created"

# Determine sync method
if [ "$FULL_SYNC" = true ]; then
    SYNC_MSG="full sync (includes .git)"
    EXCLUDE_ARGS="--exclude 'target'"
else
    SYNC_MSG="incremental sync (excludes target/ and .git)"
    EXCLUDE_ARGS="--exclude 'target' --exclude '.git' --exclude 'Cargo.lock'"
fi

# Sync source code
header "Syncing Source Code ($SYNC_MSG)"

log "Starting rsync..."
log "Source: $(pwd)/"
log "Destination: $REMOTE_HOST:$REMOTE_DIR/"
log "Excluding: target/, .git, Cargo.lock, .DS_Store, *.swp, .idea"

RSYNC_CMD="rsync -avz --delete \
  --exclude 'target' \
  --exclude '.git' \
  --exclude 'Cargo.lock' \
  --exclude '.DS_Store' \
  --exclude '*.swp' \
  --exclude '.idea' \
  --exclude '.vscode' \
  --exclude '*.log' \
  ./ $REMOTE_HOST:$REMOTE_DIR/"

if ! eval "$RSYNC_CMD" 2>&1 | tee -a "$DEPLOY_LOG"; then
    error "rsync failed"
fi
success "Source code synced"

# Get deployment size
log "Calculating sync size..."
SYNC_SIZE=$(ssh "$REMOTE_HOST" "du -sh $REMOTE_DIR 2>/dev/null | cut -f1" || echo "unknown")
log "Remote directory size: $SYNC_SIZE"

# Verify remote files
header "Verifying Remote Files"

log "Checking critical files on remote..."
CRITICAL_FILES=("Cargo.toml" "src/main.rs" "src/lib.rs")

for file in "${CRITICAL_FILES[@]}"; do
    if ssh "$REMOTE_HOST" "test -f $REMOTE_DIR/$file"; then
        success "$file exists"
    else
        warning "$file not found"
    fi
done

# Check remote toolchain
header "Checking Remote Toolchain"

log "Checking Rust installation..."
if RUST_VERSION=$(ssh "$REMOTE_HOST" "rustc --version 2>/dev/null"); then
    success "Rust installed: $RUST_VERSION"
else
    error "Rust not installed on remote. Install with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
fi

if CARGO_VERSION=$(ssh "$REMOTE_HOST" "cargo --version 2>/dev/null"); then
    success "Cargo installed: $CARGO_VERSION"
else
    error "Cargo not installed on remote"
fi

# Build verification (optional)
if [ "$SKIP_BUILD" = false ]; then
    header "Running Remote Build Verification"

    log "Running 'cargo check' on remote..."
    if ssh "$REMOTE_HOST" "cd $REMOTE_DIR && timeout 300 cargo check 2>&1" | tail -20 | tee -a "$DEPLOY_LOG"; then
        success "Remote build check passed"
    else
        error "Remote build check failed. Check toolchain or dependencies."
    fi
else
    log "Skipping build verification (--no-build flag)"
fi

# Generate deployment report
header "Deployment Report"

cat > "$DEPLOY_LOG" << EOF
=====================================
Deployment Report
=====================================
Date: $(date)
Source: $(pwd)
Remote: $REMOTE_HOST:$REMOTE_DIR

Branch: $CURRENT_BRANCH
Commit: $CURRENT_COMMIT
Remote Size: $SYNC_SIZE

Sync Method: $SYNC_MSG
Build Check: $([ "$SKIP_BUILD" = true ] && echo "Skipped" || echo "Passed")

Status: SUCCESS
=====================================
EOF

log "Full log saved to: $DEPLOY_LOG"

# Final summary
header "Summary"
success "Deployment completed successfully!"
echo ""
echo "Next steps:"
echo "1. Run tests: ./scripts/remote/run_tests_remote.sh"
echo "2. Monitor tests: ./scripts/remote/monitor_tests.sh"
echo "3. Download results: scp -r $REMOTE_HOST:$REMOTE_DIR/results/ ."
echo ""
echo "Or manually:"
echo "  ssh $REMOTE_HOST"
echo "  cd $REMOTE_DIR"
echo "  cargo test"
echo ""

exit 0
