"""
Profile memory consumption when creating large batches of DataPoint objects.
"""
from __future__ import annotations

import argparse
import gc
import tracemalloc
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Dict, List, Tuple

import rust_daq


@dataclass
class PythonDataPoint:
    timestamp: datetime
    channel: str
    value: float
    unit: str
    metadata: Dict[str, object] | None = None


def make_rust_points(count: int) -> List[rust_daq.DataPoint]:
    return [
        rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel=f"ch{idx % 4}",
            value=idx * 0.01,
            unit="V",
            metadata={"sequence": idx},
        )
        for idx in range(count)
    ]


def make_python_points(count: int) -> List[PythonDataPoint]:
    return [
        PythonDataPoint(
            timestamp=datetime.now(timezone.utc),
            channel=f"ch{idx % 4}",
            value=idx * 0.01,
            unit="V",
            metadata={"sequence": idx},
        )
        for idx in range(count)
    ]


SIZES = (1_000, 10_000, 100_000)
CASES = {
    "Rust Binding": make_rust_points,
    "Python Dataclass": make_python_points,
}


def profile_memory(label: str, count: int) -> Tuple[int, int]:
    factory = CASES[label]
    gc.collect()
    tracemalloc.start()
    points = factory(count)
    current, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    # Keep reference alive so GC does not free before measurement.
    _ = points
    return current, peak


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3, help="Number of repetitions per measurement")
    args = parser.parse_args()

    print("size | variant | current_bytes | peak_bytes | avg_current | avg_peak")
    print("-" * 80)
    for size in SIZES:
        for label in CASES:
            currents: List[int] = []
            peaks: List[int] = []
            for _ in range(args.runs):
                current, peak = profile_memory(label, size)
                currents.append(current)
                peaks.append(peak)
            avg_current = sum(currents) / len(currents)
            avg_peak = sum(peaks) / len(peaks)
            print(
                f"{size:>5} | {label:<16} | {min(currents):>13} | {max(peaks):>10} | {avg_current:>11.0f} | {avg_peak:>8.0f}"
            )


if __name__ == "__main__":
    main()
