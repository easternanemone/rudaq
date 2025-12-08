#!/bin/bash
# Baseline performance benchmark for rust-daq
# Measures GUI fps, memory usage, and broadcast lag before refactoring

set -e

BASELINE_FILE="baseline_metrics.json"
CARGO_CMD="cargo run --release"

echo "=== rust-daq Baseline Performance Benchmark ==="
echo "This script measures current performance to validate Phase 1 improvements"
echo ""

# Function to get process memory (macOS)
get_memory_mb() {
    local pid=$1
    # macOS: use ps with RSS in KB, convert to MB
    ps -o rss= -p "$pid" 2>/dev/null | awk '{print $1/1024}' || echo "0"
}

# Build in release mode
echo "[1/5] Building rust-daq in release mode..."
cargo build --release --quiet || {
    echo "ERROR: Build failed"
    exit 1
}

echo "[2/5] Starting rust-daq in background..."
$CARGO_CMD > /tmp/daq_benchmark.log 2>&1 &
DAQ_PID=$!

# Wait for app to start
sleep 5

if ! kill -0 $DAQ_PID 2>/dev/null; then
    echo "ERROR: rust-daq failed to start"
    cat /tmp/daq_benchmark.log
    exit 1
fi

echo "[3/5] Measuring baseline metrics..."

# Initial memory
INITIAL_MEM=$(get_memory_mb $DAQ_PID)
echo "  Initial memory: ${INITIAL_MEM} MB"

# Let app stabilize
sleep 2

# Measure memory after startup
STARTUP_MEM=$(get_memory_mb $DAQ_PID)
echo "  Startup memory: ${STARTUP_MEM} MB"

# Count RecvError::Lagged occurrences in logs
LAGGED_COUNT=$(grep -c "lagged by" /tmp/daq_benchmark.log 2>/dev/null || echo "0")
echo "  Broadcast lag events: ${LAGGED_COUNT}"

# Note: GUI fps measurement requires instrumentation in the app
# For now, we'll just record that it needs to be measured manually
echo "  GUI fps: [Manual measurement required - run with RUST_LOG=debug]"

echo "[4/5] Cleaning up..."
kill $DAQ_PID 2>/dev/null || true
sleep 2

echo "[5/5] Writing baseline metrics..."

# Write JSON baseline
cat > "$BASELINE_FILE" << EOF
{
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "git_commit": "$(git rev-parse HEAD 2>/dev/null || echo 'unknown')",
  "metrics": {
    "memory_initial_mb": ${INITIAL_MEM},
    "memory_startup_mb": ${STARTUP_MEM},
    "broadcast_lag_count": ${LAGGED_COUNT},
    "gui_fps": null,
    "notes": "GUI fps requires manual measurement with debug logging. Run with RUST_LOG=debug and count frame updates."
  },
  "configuration": {
    "broadcast_channel_capacity": 1024,
    "command_channel_capacity": 32,
    "test_description": "Baseline measurement before Phase 0 quick wins"
  }
}
EOF

echo ""
echo "=== Baseline Metrics Saved ==="
cat "$BASELINE_FILE"
echo ""
echo "Baseline file: $BASELINE_FILE"
echo ""
echo "NOTE: To measure GUI fps accurately:"
echo "  1. Run: RUST_LOG=debug cargo run --release"
echo "  2. Open GUI and perform instrument spawn/stop operations"
echo "  3. Count 'Frame rendered' or similar log messages per second"
echo "  4. Update gui_fps field in $BASELINE_FILE manually"
echo ""
echo "For Phase 1 validation, rerun this script and compare metrics."
