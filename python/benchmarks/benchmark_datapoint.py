"""
Benchmark DataPoint construction between Rust bindings and pure Python options.

Run directly:
    python python/benchmarks/benchmark_datapoint.py --samples 50000
"""
from __future__ import annotations

import argparse
import math
import statistics as stats
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Callable, Dict, List, Tuple

import rust_daq


@dataclass
class PythonDataPoint:
    timestamp: datetime
    channel: str
    value: float
    unit: str
    metadata: Dict[str, object] | None = None


def percentile(values: List[float], pct: float) -> float:
    if not values:
        return float("nan")
    ordered = sorted(values)
    rank = (len(ordered) - 1) * pct
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return ordered[int(rank)]
    weight = rank - lower
    return ordered[lower] * (1 - weight) + ordered[upper] * weight


def record_latencies(samples: int, func: Callable[[], object]) -> List[float]:
    durations: List[float] = []
    for _ in range(samples):
        start = time.perf_counter_ns()
        func()
        end = time.perf_counter_ns()
        durations.append((end - start) / 1_000.0)  # microseconds
    return durations


def format_metrics(label: str, durations_us: List[float]) -> Tuple[str, List[str]]:
    total_time_s = sum(durations_us) / 1_000_000.0
    ops_per_sec = len(durations_us) / total_time_s if total_time_s else float("inf")
    avg = stats.fmean(durations_us)
    p50 = percentile(durations_us, 0.50)
    p95 = percentile(durations_us, 0.95)
    p99 = percentile(durations_us, 0.99)
    return (
        label,
        [
            f"{ops_per_sec:>12.0f}",
            f"{avg:>10.3f}",
            f"{p50:>10.3f}",
            f"{p95:>10.3f}",
            f"{p99:>10.3f}",
        ],
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=50_000, help="Number of DataPoints to create per variant")
    args = parser.parse_args()

    metadata = {"source": "benchmark", "sequence": 0}

    def make_rust_datapoint() -> rust_daq.DataPoint:
        return rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="ch1",
            value=42.0,
            unit="V",
            metadata=metadata,
        )

    def make_python_dict() -> Dict[str, object]:
        return {
            "timestamp": datetime.now(timezone.utc),
            "channel": "ch1",
            "value": 42.0,
            "unit": "V",
            "metadata": metadata,
        }

    def make_python_dataclass() -> PythonDataPoint:
        return PythonDataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="ch1",
            value=42.0,
            unit="V",
            metadata=metadata,
        )

    cases: List[Tuple[str, Callable[[], object]]] = [
        ("Rust Binding", make_rust_datapoint),
        ("Python Dict", make_python_dict),
        ("Python Dataclass", make_python_dataclass),
    ]

    rows: List[Tuple[str, List[str]]] = []
    for label, func in cases:
        durations = record_latencies(args.samples, func)
        rows.append(format_metrics(label, durations))

    headers = ["Variant", "ops/sec", "avg_us", "p50_us", "p95_us", "p99_us"]
    print(" | ".join(headers))
    print("-" * 70)
    for label, metrics in rows:
        print(" | ".join([label.ljust(14)] + metrics))


if __name__ == "__main__":
    main()
