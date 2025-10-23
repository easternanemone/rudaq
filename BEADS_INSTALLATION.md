# Beads Installation Guide for rust-daq

This document describes the beads installation setup for the rust-daq project and provides guidance for installing in different environments.

## Current Environment Status

This environment has **partial beads installation** due to network restrictions:

✅ **Installed Successfully:**
- `beads-mcp` (v0.11.0) - MCP server for Claude Code integration
- All Python dependencies (fastmcp, pydantic, etc.)
- `cffi` for cryptography support

⚠️ **Installation Limitation:**
- `bd` CLI tool - **Stub version only**
- Full installation blocked by network restrictions preventing Go module downloads
- modernc.org/sqlite dependency cannot be fetched (403 Forbidden)

## What Works in This Environment

The beads-mcp MCP server is fully functional and can be used with Claude Code. However, it will fail when attempting to execute `bd` CLI commands because only a stub script is installed.

The stub script provides helpful error messages and installation instructions when invoked.

## Full Installation (For Environments with Network Access)

### Prerequisites

- Go 1.24+ (for bd CLI)
- Python 3.8+ (for beads-mcp)
- Network access to:
  - proxy.golang.org (Go module proxy)
  - pypi.org (Python packages)
  - github.com (source repository)

### Step 1: Install bd CLI

Choose one of the following methods:

#### Option A: Quick Install Script (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/steveyegge/beads/main/scripts/install.sh | bash

# Verify installation
bd version
```

#### Option B: Using Go

```bash
# Install via go install
go install github.com/steveyegge/beads/cmd/bd@latest

# Add to PATH if needed
export PATH="$PATH:$(go env GOPATH)/bin"

# Verify installation
bd version
```

#### Option C: Homebrew (macOS/Linux)

```bash
brew tap steveyegge/beads
brew install bd

# Verify installation
bd version
```

#### Option D: Build from Source

```bash
# Clone repository
git clone https://github.com/steveyegge/beads
cd beads

# Build binary
go build -o bd ./cmd/bd

# Install to system
sudo mv bd /usr/local/bin/

# Verify installation
bd version
```

### Step 2: Install beads-mcp (MCP Server)

```bash
# Using pip
pip install beads-mcp

# Or using uv (recommended)
uv tool install beads-mcp

# Verify installation
beads-mcp --help
```

### Step 3: Install cffi (if needed)

```bash
# Required for cryptography support
pip install cffi
```

## Initialization in rust-daq Project

Once bd CLI is fully installed:

```bash
# Navigate to rust-daq project
cd /path/to/rust-daq

# Initialize beads with 'daq' prefix
bd init --prefix daq

# Verify initialization
bd list

# Set environment variables (optional but recommended)
export BEADS_DB=.beads/daq.db
export BD_ACTOR="your-name-or-agent-name"
```

## Troubleshooting

### `bd: command not found` (after installation)

```bash
# Check installation
which bd

# Add Go bin to PATH
export PATH="$PATH:$(go env GOPATH)/bin"

# Make permanent by adding to ~/.bashrc or ~/.zshrc
echo 'export PATH="$PATH:$(go env GOPATH)/bin"' >> ~/.bashrc
```

### Go module download failures

```bash
# Try with direct proxy
GOPROXY=direct go install github.com/steveyegge/beads/cmd/bd@latest

# Or set proxy explicitly
export GOPROXY=https://proxy.golang.org,direct
go install github.com/steveyegge/beads/cmd/bd@latest
```

### `database is locked` errors

```bash
# Find and kill hanging bd processes
ps aux | grep bd
kill <pid>

# Remove lock files (only if no bd processes running)
rm .beads/*.db-journal .beads/*.db-wal .beads/*.db-shm
```

### beads-mcp import errors

```bash
# Install missing dependencies
pip install cffi authlib

# Reinstall beads-mcp
pip install --force-reinstall beads-mcp
```

## Network-Restricted Environment Workaround

In environments where Go module downloads are blocked (like this one):

1. **Pre-build on a machine with network access:**
   ```bash
   # On a machine with network access
   git clone https://github.com/steveyegge/beads
   cd beads
   go build -o bd ./cmd/bd

   # Copy binary to restricted environment
   scp bd user@restricted-machine:/usr/local/bin/
   ```

2. **Use vendor mode:**
   ```bash
   # On a machine with network access
   git clone https://github.com/steveyegge/beads
   cd beads
   go mod vendor

   # Create tarball including vendor directory
   tar -czf beads-vendored.tar.gz .

   # On restricted machine
   tar -xzf beads-vendored.tar.gz
   cd beads
   go build -mod=vendor -o bd ./cmd/bd
   sudo mv bd /usr/local/bin/
   ```

3. **Use pre-built binaries:**
   - Check GitHub releases for pre-compiled binaries
   - Download appropriate binary for your platform
   - Install to /usr/local/bin or other PATH location

## Environment Variables

Recommended environment variables for rust-daq:

```bash
# Point to project-local database
export BEADS_DB=.beads/daq.db

# Set actor name for audit trail
export BD_ACTOR="claude-agent"  # or your name

# Enable debug logging (optional)
export BD_DEBUG=1

# For beads-mcp
export BEADS_USE_DAEMON=1  # Use daemon RPC instead of CLI
export BEADS_WORKING_DIR=/path/to/rust-daq
```

## Testing Installation

After successful installation, test with:

```bash
# Test bd CLI
bd version
bd quickstart

# Test beads-mcp
beads-mcp --help

# Initialize in rust-daq
cd /path/to/rust-daq
bd init --prefix daq
bd create "Test issue" -t task -p 2
bd list
bd ready
```

## Resources

- **Main documentation**: https://github.com/steveyegge/beads
- **Quick start guide**: `bd quickstart`
- **MCP server docs**: https://github.com/steveyegge/beads/tree/main/integrations/beads-mcp
- **Agent integration**: See beads AGENTS.md for AI workflow patterns

## Current Installation Summary

**This Environment:**
- ✅ beads-mcp v0.11.0 installed and functional
- ✅ All Python dependencies installed
- ⚠️ bd CLI stub only (network restriction workaround)
- 📝 Full installation requires network access or pre-built binary

**Next Steps for Full Functionality:**
1. Install bd CLI using one of the methods above (requires network access)
2. Initialize beads in rust-daq: `bd init --prefix daq`
3. Configure environment variables
4. Start using beads for issue tracking
