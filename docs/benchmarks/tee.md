# Tee Pipeline Benchmark

Synthetic benchmark for the Tee (reliable mpsc + lossy broadcast) path.

Commands:
```
TEE_BENCH_MESSAGES=100000 TEE_BENCH_LATENCY=1 cargo run -p common --example tee_bench --release
TEE_BENCH_MESSAGES=50000 TEE_BENCH_BUFFER=32 cargo run -p common --example tee_bench --release
```

Latest run (2025-12-12, Apple M-series laptop):
- Messages: 100,000 (buffer 1024), payload 0 bytes  
  Throughput: ~2.35 M msgs/s  
  Latency p50/p90/p99/max: 416 / 440 / 475 / 504 µs

- Messages: 50,000 (buffer 32), payload 0 bytes  
  Throughput: ~2.03 M msgs/s  
  Latency p50/p90/p99/max: 8 / 16 / 19 / 71 µs

Notes:
- `TEE_BENCH_PAYLOAD` adds bytes to the measurement name to simulate metadata size.
- Set `TEE_BENCH_LATENCY=0` to skip percentile computation.
- Reliable path latencies are computed from measurement timestamps to receiver time; clocks are monotonic on a single host.
- CI publishes sample outputs (including backpressure case) as artifact `tee-bench-<sha>`; see GitHub Actions run for the latest logs.

## RingBuffer writer micro-bench

Command (uses /tmp by default; override `RING_BENCH_PATH`):
```
RING_BENCH_MESSAGES=20000 RING_BENCH_BUFFER_MB=16 cargo run -p daq-storage --example ring_writer_bench --release
```

Latest run (2025-12-12, Apple M-series laptop):
- 20,000 frames in 0.004s → ~4.73 M writes/s (buffer 16 MB, payload 0 bytes, path /tmp/ring_writer_bench.buf)

## Arrow -> RingBuffer bench

Requires `storage_arrow` feature:
```
RING_ARROW_MESSAGES=2000 RING_ARROW_ROWS=1000 RING_ARROW_BUFFER_MB=32 cargo run -p daq-storage --example ring_arrow_bench --release --features storage_arrow
```

Latest run (2025-12-12, Apple M-series laptop):
- 2,000 batches (1,000 rows each) in 0.025s → ~80,160 batches/s (buffer 32 MB, path /tmp/ring_arrow_bench.buf)
