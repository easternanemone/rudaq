#!/usr/bin/env bash
#
# Lightweight Regression Harness for Optimization Claims (bd-pman.1.4)
# Run this script before and after your optimization to compare baseline metrics.

set -euo pipefail

echo "=========================================================="
echo "rust-daq Optimization Baseline Harness"
echo "=========================================================="

echo "[1/4] Measuring Storage Backend (Arrow Tensor) Write Path..."
# Run storage_arrow benchmarks
cargo bench -p storage --bench arrow_tensor --features storage_arrow | grep -E "time:|thrpt:|arrow_write|arrow_tensor" || true
echo ""

echo "[2/4] Measuring Ring Buffer Throughput..."
cargo test bench_ring_buffer --release -p storage -- --nocapture | awk '/--- RING BUFFER BENCHMARK RESULTS ---/{flag=1} /--- END BENCHMARK RESULTS ---/{print; flag=0} flag' || true
echo ""

echo "[3/4] Measuring Tap Registry Routing Cost..."
cargo test bench_tap_registry --release -p storage -- --nocapture | awk '/bench: tap_registry_throughput/{flag=1; count=0} flag {print; count++} count==9{flag=0}' || true
echo ""

echo "[4/4] Measuring Mock Daemon Startup Latency..."
# Use our standalone measurement script for mock mode
if [ -x "scripts/ops/measure-startup.sh" ]; then
    ./scripts/ops/measure-startup.sh mock | grep -E "Ready in|Registrations|Database" || true
else
    echo "Warning: scripts/ops/measure-startup.sh not found."
fi

echo "=========================================================="
echo "Done! Save this output as your 'before' state, apply your"
echo "optimizations, then run again to prove performance gains."
echo "=========================================================="
