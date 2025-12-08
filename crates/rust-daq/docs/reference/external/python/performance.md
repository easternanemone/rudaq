# Python Binding Performance Benchmarks

This document describes the benchmark suite for the `rust-daq` Python bindings
and explains how to interpret the results. The suite demonstrates the overhead
of crossing the Rust↔Python boundary, compares the bindings with idiomatic
Python implementations, and evaluates sustained throughput in a 1 kHz data
acquisition scenario.

## Benchmark Layout

```
python/
├── benches/
│   └── datapoint_bench.rs        # Criterion micro-benchmarks (Rust)
├── benchmarks/
│   ├── benchmark_datapoint.py    # DataPoint creation latency & ops/sec
│   ├── benchmark_throughput.py   # Batch creation throughput (1k/10k/100k)
│   ├── benchmark_memory.py       # Traced memory usage per implementation
│   └── compare_alternatives.py   # Orchestrated comparison + pipeline sim
└── docs/
    └── performance.md            # This guide
```

## Methodology

### Rust Criterion Suite

The Criterion benchmarks (`python/benches/datapoint_bench.rs`) execute entirely
in Rust and evaluate:

- `datapoint_rust_to_py_struct`: cost of converting a core `rust_daq::DataPoint`
  into the exported `PyDataPoint` struct.
- `datapoint_roundtrip_rust_python_rust`: conversion from Rust → Python object →
  Rust struct via attribute extraction.
- `batch_roundtrip`: batched conversions (1, 100, 1,000 elements) to quantify
  aggregate throughput across the Python boundary.

Criterion reports include: iterations per second, latency distributions, and
HTML reports (enabled via `criterion`'s `html_reports` feature).

### Python Micro & Macro Benchmarks

The Python scripts measure real-world behaviour when the bindings are used from
Python code:

- **Creation latency (`benchmark_datapoint.py`)** – Compares the bindings with a
  plain dictionary and a Python `dataclass`, reporting ops/sec and latency
  percentiles (p50, p95, p99).
- **Throughput (`benchmark_throughput.py`)** – Creates batches of 1k, 10k, and
  100k points, tracking sustained points/sec, median, average, and standard
  deviation across multiple runs.
- **Memory profile (`benchmark_memory.py`)** – Uses `tracemalloc` to capture
  current and peak Python-side allocations for each batch size.
- **Comparison suite (`compare_alternatives.py`)** – Aggregates the above and
  simulates a 1 kHz acquisition pipeline with a moving-average processor. The
  script can emit JSON summaries and (optionally) throughput plots when
  `matplotlib` is available.

All Python benchmarks rely on the same construction patterns to keep metadata
and timestamp handling comparable between implementations.

## Running the Benchmarks

1. **Rust micro-benchmarks**
   ```bash
   cargo bench --manifest-path python/Cargo.toml
   ```
   Criterion reports will be written to `python/target/criterion/`.

2. **Python scripts**
   ```bash
   # Creation latency
   python python/benchmarks/benchmark_datapoint.py --samples 50000

   # Throughput
   python python/benchmarks/benchmark_throughput.py --repeats 7

   # Memory usage
   python python/benchmarks/benchmark_memory.py --runs 5

   # Full comparison with JSON export and plots
   python python/benchmarks/compare_alternatives.py \
       --samples 40000 \
       --repeats 7 \
       --runs 5 \
       --rate 1000 \
       --duration 15 \
       --json python/benchmarks/results/latest.json \
       --plot-dir python/benchmarks/results/
   ```

   > **Note**: Install optional dependencies (e.g. `matplotlib`) in your Python
   > environment to enable plot generation.

## Result Interpretation

- **Ops/sec & Latency percentiles** show the marginal overhead per DataPoint.
  Expect the Rust binding to outperform pure Python constructors, especially in
  tail latencies (p95/p99).
- **Throughput** reveals scaling behaviour for batch creation. The Rust binding
  should maintain near-linear scaling, while pure Python may exhibit steeper
  slowdown beyond 10k objects.
- **Memory pressure** highlights allocation efficiency. Lower `peak_bytes` for
  Rust indicates tighter integration with Rust's allocator via PyO3.
- **1 kHz pipeline** measures end-to-end capability (data creation + moving
  average processing). The effective rate must exceed the target acquisition
  frequency to maintain headroom for additional processing stages.

Capture hardware details (CPU model, core count, RAM, Python version, and
operating system) alongside benchmark output before sharing results.

## Performance Tuning Guidance

- **Batch conversions** – Prefer converting collections in Rust and exposing
  memory-friendly structures (e.g. `PyList` of `PyDataPoint`) to minimise GIL
  contention.
- **Metadata management** – Reuse metadata dictionaries when possible to avoid
  repeated allocations during high-frequency acquisition.
- **Pipeline design** – Keep hot loops in Rust processors or NumPy arrays; avoid
  per-point Python callbacks in critical paths.
- **GIL considerations** – Release the GIL in long-running Rust tasks and
  consider batching callbacks across threads to reduce context switches.

## Known Bottlenecks & Mitigations

| Area | Observation | Mitigation |
| ---- | ----------- | ---------- |
| GIL acquisition | Roundtrip benchmarks show GIL acquisition dominates tail latency. | Move heavy processing to Rust, or use `pyo3` `PyAllowThreads` around Rust loops that do not touch Python objects. |
| Timestamp creation | Python `datetime.now()` appears in every constructor and is relatively expensive. | Allow Rust to stamp timestamps (UTC) and expose them over the binding to remove Python-side clock calls. |
| Metadata copies | Large metadata payloads increase allocations across the boundary. | Serialize metadata once in Rust and share references via `Py<PyAny>` or structured numpy arrays when feasible. |
| Memory snapshots | `tracemalloc` overhead inflates latency during measurement. | Reserve `tracemalloc` for diagnostic runs; disable it in production tests. |

## NEXT STEPS

- Automate result archival in CI artifacts (see `.github/workflows/benchmarks.yml`).
- Extend pipeline simulations to include FFT/IIR processors and storage writers
  as those components gain Python-exposed hooks.
- Integrate regression thresholds (e.g. compare against previous run medians)
  once a baseline dataset is established.
