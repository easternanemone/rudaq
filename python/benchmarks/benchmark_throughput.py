"""
Measure sustained throughput for DataPoint creation in different implementations.
"""
from __future__ import annotations

import argparse
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


BenchmarkFunc = Callable[[int], None]


def create_rust_points(count: int) -> None:
    _ = [
        rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel=f"ch{idx % 4}",
            value=idx * 0.01,
            unit="V",
            metadata={"sequence": idx},
        )
        for idx in range(count)
    ]


def create_python_points(count: int) -> None:
    _ = [
        PythonDataPoint(
            timestamp=datetime.now(timezone.utc),
            channel=f"ch{idx % 4}",
            value=idx * 0.01,
            unit="V",
            metadata={"sequence": idx},
        )
        for idx in range(count)
    ]


CASES: Dict[str, BenchmarkFunc] = {
    "Rust Binding": create_rust_points,
    "Python Dataclass": create_python_points,
}


SIZES = (1_000, 10_000, 100_000)


def measure(case: str, func: BenchmarkFunc, count: int, repeats: int) -> Tuple[float, List[float]]:
    durations: List[float] = []
    for _ in range(repeats):
        start = time.perf_counter()
        func(count)
        end = time.perf_counter()
        durations.append(end - start)
    median = stats.median(durations)
    throughput = count / median if median else float("inf")
    return throughput, durations


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=5, help="Repetitions per size variant")
    args = parser.parse_args()

    print("size | variant | throughput_pts_per_s | median_s | avg_s | stdev_s")
    print("-" * 80)
    for size in SIZES:
        for label, func in CASES.items():
            throughput, durations = measure(label, func, size, args.repeats)
            avg = stats.fmean(durations)
            stdev = stats.pstdev(durations) if len(durations) > 1 else 0.0
            median = stats.median(durations)
            print(
                f"{size:>5} | {label:<16} | {throughput:>20.0f} | {median:>8.4f} | {avg:>7.4f} | {stdev:>7.4f}"
            )


if __name__ == "__main__":
    main()
