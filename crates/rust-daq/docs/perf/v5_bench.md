# V5 Performance Benchmarks (bd-vms4.1)

_Last updated: 2025-12-07_

## Scope
- Parameter<T> set/get latency (f64, bool, String)
- Hardware callback overhead (mock async writer)
- Run with `cargo bench --bench parameter_bench`

## Test Environment
- CPU / RAM / OS: (local dev machine, not recorded)
- Rust toolchain: stable (cargo bench default)
- Features enabled: default

## How to Run
```bash
cargo bench --bench parameter_bench
# HTML reports (criterion): target/criterion/report/index.html
```

## Results
- p50 / p90 / p99 latencies (ns or µs)
- Notes on regressions (fail >10% vs last baseline)

| Benchmark | p50 | p90 | p99 | Notes |
|-----------|-----|-----|-----|-------|
| set_f64 | 85.5 ns | 85.9 ns | 86.0 ns | |
| set_bool | 85.7 ns | 86.1 ns | 86.3 ns | |
| set_string | 104.2 ns | 104.8 ns | 105.0 ns | |
| get_f64 | 9.92 ns | 10.0 ns | 10.1 ns | |
| get_bool | 9.96 ns | 10.0 ns | 10.1 ns | |
| get_string | 20.9 ns | 21.1 ns | 21.3 ns | |
| set_with_callback | 98.6 ns | 99.1 ns | 99.2 ns | Mock async writer |

## Observations / Actions
- All latencies well below 0.2 µs; callback overhead adds ~13 ns over plain set.
- Plotters backend used (no gnuplot installed).
