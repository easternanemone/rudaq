#!/usr/bin/env bash
# rust-daq Demo Launcher
#
# Quick-start script to run the demo with mock hardware.
# Handles daemon startup, cleanup, and error recovery.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
DAEMON_PORT=50051
DEMO_CONFIG="config/demo.toml"
DEMO_SCRIPT="examples/demo_scan.rhai"
DAEMON_PID_FILE="/tmp/rust-daq-demo-daemon.pid"

# Cleanup function
cleanup() {
    if [ -f "$DAEMON_PID_FILE" ]; then
        echo -e "${YELLOW}Stopping daemon...${NC}"
        DAEMON_PID=$(cat "$DAEMON_PID_FILE")
        kill "$DAEMON_PID" 2>/dev/null || true
        rm -f "$DAEMON_PID_FILE"
        echo -e "${GREEN}✓ Daemon stopped${NC}"
    fi
}

# Register cleanup on exit
trap cleanup EXIT INT TERM

# Check if we're in the rust-daq directory
if [ ! -f "Cargo.toml" ] || ! grep -q "rust-daq" Cargo.toml 2>/dev/null; then
    echo -e "${RED}Error: Run this script from the rust-daq root directory${NC}"
    exit 1
fi

# Check if demo config exists
if [ ! -f "$DEMO_CONFIG" ]; then
    echo -e "${RED}Error: Demo config not found: $DEMO_CONFIG${NC}"
    echo -e "${YELLOW}Hint: Make sure config/demo.toml exists${NC}"
    exit 1
fi

echo -e "${BLUE}╔════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║      rust-daq Demo Launcher            ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════╝${NC}"
echo ""

# Build binaries
echo -e "${YELLOW}Building rust-daq-daemon...${NC}"
cargo build --bin rust-daq-daemon --quiet 2>&1 | grep -v "^warning" || true
echo -e "${GREEN}✓ Build complete${NC}"
echo ""

# Start daemon in background
echo -e "${YELLOW}Starting daemon (port $DAEMON_PORT)...${NC}"
# Explicit --hardware-config overrides the hybrid-db default, keeping demo in mock mode
cargo run --bin rust-daq-daemon --quiet -- daemon \
    --hardware-config "$DEMO_CONFIG" \
    --port "$DAEMON_PORT" \
    > /tmp/rust-daq-demo-daemon.log 2>&1 &

DAEMON_PID=$!
echo $DAEMON_PID > "$DAEMON_PID_FILE"

# Wait for daemon to be ready
echo -e "${YELLOW}Waiting for daemon to start...${NC}"
MAX_WAIT=10
for i in $(seq 1 $MAX_WAIT); do
    if lsof -Pi :$DAEMON_PORT -sTCP:LISTEN -t >/dev/null 2>&1; then
        echo -e "${GREEN}✓ Daemon ready${NC}"
        break
    fi
    if [ $i -eq $MAX_WAIT ]; then
        echo -e "${RED}✗ Daemon failed to start within ${MAX_WAIT}s${NC}"
        echo -e "${YELLOW}Check logs: tail -f /tmp/rust-daq-demo-daemon.log${NC}"
        exit 1
    fi
    sleep 1
done

echo ""
echo -e "${GREEN}╔════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║  Daemon Running - Demo Ready!          ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════╝${NC}"
echo ""
echo -e "Daemon logs: ${BLUE}/tmp/rust-daq-demo-daemon.log${NC}"
echo -e "gRPC endpoint: ${BLUE}http://127.0.0.1:$DAEMON_PORT${NC}"
echo ""

# Present options
echo "What would you like to do?"
echo ""
echo "  ${GREEN}1)${NC} Run demo scan script (automated)"
echo "  ${GREEN}2)${NC} Launch GUI (interactive)"
echo "  ${GREEN}3)${NC} Keep daemon running (connect manually)"
echo "  ${GREEN}4)${NC} Exit"
echo ""
read -p "Choose an option [1-4]: " choice

case $choice in
    1)
        echo ""
        echo -e "${YELLOW}Running demo scan script...${NC}"
        echo ""
        cargo run --bin rust-daq-daemon --quiet -- run "$DEMO_SCRIPT"
        echo ""
        echo -e "${GREEN}✓ Demo scan complete!${NC}"
        ;;
    2)
        echo ""
        echo -e "${YELLOW}Launching GUI...${NC}"
        echo -e "${BLUE}Connect to: http://127.0.0.1:$DAEMON_PORT${NC}"
        echo ""
        cargo run --bin rust-daq-gui --features networking
        ;;
    3)
        echo ""
        echo -e "${GREEN}Daemon running in background${NC}"
        echo ""
        echo "Connect using:"
        echo "  - GUI:    ${BLUE}cargo run --bin rust-daq-gui --features networking${NC}"
        echo "  - Script: ${BLUE}cargo run --bin rust-daq-daemon -- run <script.rhai>${NC}"
        echo "  - Python: ${BLUE}import grpc; channel = grpc.insecure_channel('localhost:$DAEMON_PORT')${NC}"
        echo ""
        echo "Press ENTER to stop daemon and exit..."
        read
        ;;
    4)
        echo -e "${YELLOW}Exiting...${NC}"
        ;;
    *)
        echo -e "${RED}Invalid choice. Exiting.${NC}"
        ;;
esac

echo ""
echo -e "${YELLOW}Next steps:${NC}"
echo "  • Read DEMO.md for detailed guide"
echo "  • Try other scripts: ${BLUE}ls examples/*.rhai${NC}"
echo "  • Modify ${BLUE}$DEMO_CONFIG${NC} to customize mock devices"
echo "  • See ${BLUE}docs/guides/${NC} for scripting and storage tutorials"
echo ""
