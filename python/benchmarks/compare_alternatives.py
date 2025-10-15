"""
Run a comprehensive comparison suite across the Rust bindings and pure Python
alternatives. The script consolidates micro-benchmarks, throughput tests, memory
profiles, and a simulated 1 kHz acquisition pipeline.
"""
from __future__ import annotations

import argparse
import json
import math
import statistics as stats
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Dict, List, Optional, Tuple

import rust_daq

try:  # Optional plotting support
    import matplotlib.pyplot as plt  # type: ignore
except Exception:  # pragma: no cover - optional dependency
    plt = None


@dataclass
class PythonDataPoint:
    timestamp: datetime
    channel: str
    value: float
    unit: str
    metadata: Dict[str, object] | None = None


# ---------------------------------------------------------------------------
# Helper utilities
# ---------------------------------------------------------------------------

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


def record_latencies(samples: int, factory: Callable[[], object]) -> List[float]:
    durations: List[float] = []
    for _ in range(samples):
        start = time.perf_counter_ns()
        factory()
        end = time.perf_counter_ns()
        durations.append((end - start) / 1_000.0)
    return durations


# ---------------------------------------------------------------------------
# Micro benchmark: DataPoint creation
# ---------------------------------------------------------------------------

def run_creation_bench(samples: int) -> Dict[str, Dict[str, float]]:
    metadata = {"source": "compare", "sequence": 0}

    def rust_factory() -> rust_daq.DataPoint:
        return rust_daq.DataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="ch1",
            value=42.0,
            unit="V",
            metadata=metadata,
        )

    def dict_factory() -> Dict[str, object]:
        return {
            "timestamp": datetime.now(timezone.utc),
            "channel": "ch1",
            "value": 42.0,
            "unit": "V",
            "metadata": metadata,
        }

    def dataclass_factory() -> PythonDataPoint:
        return PythonDataPoint(
            timestamp=datetime.now(timezone.utc),
            channel="ch1",
            value=42.0,
            unit="V",
            metadata=metadata,
        )

    cases = {
        "rust_binding": rust_factory,
        "python_dict": dict_factory,
        "python_dataclass": dataclass_factory,
    }

    results: Dict[str, Dict[str, float]] = {}
    for label, factory in cases.items():
        durations = record_latencies(samples, factory)
        total_time_s = sum(durations) / 1_000_000.0
        ops_per_sec = samples / total_time_s if total_time_s else float("inf")
        results[label] = {
            "ops_per_sec": ops_per_sec,
            "avg_us": stats.fmean(durations),
            "p50_us": percentile(durations, 0.50),
            "p95_us": percentile(durations, 0.95),
            "p99_us": percentile(durations, 0.99),
        }
    return results


# ---------------------------------------------------------------------------
# Throughput benchmark
# ---------------------------------------------------------------------------

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


def run_throughput_bench(sizes: Tuple[int, ...], repeats: int) -> Dict[str, Dict[int, Dict[str, float]]]:
    cases: Dict[str, Callable[[int], List[object]]] = {
        "rust_binding": make_rust_points,
        "python_dataclass": make_python_points,
    }

    summary: Dict[str, Dict[int, Dict[str, float]]] = {}
    for label, factory in cases.items():
        summary[label] = {}
        for size in sizes:
            timings: List[float] = []
            for _ in range(repeats):
                start = time.perf_counter()
                _ = factory(size)
                end = time.perf_counter()
                timings.append(end - start)
            summary[label][size] = {
                "median_s": stats.median(timings),
                "avg_s": stats.fmean(timings),
                "stdev_s": stats.pstdev(timings) if len(timings) > 1 else 0.0,
                "throughput_pts_per_s": size / stats.median(timings) if stats.median(timings) else float("inf"),
            }
    return summary


# ---------------------------------------------------------------------------
# Memory benchmark
# ---------------------------------------------------------------------------

def run_memory_bench(sizes: Tuple[int, ...], runs: int) -> Dict[str, Dict[int, Dict[str, float]]]:
    import gc
    import tracemalloc

    cases = {
        "rust_binding": make_rust_points,
        "python_dataclass": make_python_points,
    }

    result: Dict[str, Dict[int, Dict[str, float]]] = {}
    for label, factory in cases.items():
        result[label] = {}
        for size in sizes:
            currents: List[int] = []
            peaks: List[int] = []
            for _ in range(runs):
                gc.collect()
                tracemalloc.start()
                points = factory(size)
                current, peak = tracemalloc.get_traced_memory()
                tracemalloc.stop()
                _ = points
                currents.append(current)
                peaks.append(peak)
            result[label][size] = {
                "current_bytes": min(currents),
                "peak_bytes": max(peaks),
                "avg_current_bytes": float(sum(currents)) / len(currents),
                "avg_peak_bytes": float(sum(peaks)) / len(peaks),
            }
    return result


# ---------------------------------------------------------------------------
# Macro benchmark: 1 kHz acquisition pipeline
# ---------------------------------------------------------------------------

def moving_average(values: List[float], window: int = 16) -> List[float]:
    if window <= 0:
        return values
    acc: List[float] = []
    running = 0.0
    for idx, value in enumerate(values):
        running += value
        if idx >= window:
            running -= values[idx - window]
        if idx >= window - 1:
            acc.append(running / window)
    return acc


def run_pipeline(factory: Callable[[int], List[object]], accessor: Callable[[object], float], rate_hz: int, duration_s: int) -> Dict[str, float]:
    total_points = rate_hz * duration_s
    start = time.perf_counter()
    points = factory(total_points)
    creation_time = time.perf_counter() - start

    values = [accessor(point) for point in points]
    pipeline_start = time.perf_counter()
    _ = moving_average(values)
    pipeline_time = time.perf_counter() - pipeline_start

    total_time = creation_time + pipeline_time
    return {
        "points": total_points,
        "creation_time_s": creation_time,
        "pipeline_time_s": pipeline_time,
        "total_time_s": total_time,
        "effective_rate": total_points / total_time if total_time else float("inf"),
    }


def run_pipeline_bench(rate_hz: int, duration_s: int) -> Dict[str, Dict[str, float]]:
    return {
        "rust_binding": run_pipeline(make_rust_points, lambda dp: dp.value, rate_hz, duration_s),
        "python_dataclass": run_pipeline(
            make_python_points, lambda dp: dp.value, rate_hz, duration_s
        ),
    }


# ---------------------------------------------------------------------------
# Reporting helpers
# ---------------------------------------------------------------------------

def maybe_write_json(path: Optional[Path], payload: Dict[str, object]) -> None:
    if not path:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def maybe_plot_throughput(path: Optional[Path], throughput: Dict[str, Dict[int, Dict[str, float]]]) -> None:
    if not path or plt is None:
        return
    path.mkdir(parents=True, exist_ok=True)
    for label, series in throughput.items():
        sizes = sorted(series.keys())
        rates = [series[size]["throughput_pts_per_s"] for size in sizes]
        plt.plot(sizes, rates, marker="o", label=label.replace("_", " ").title())
    plt.xlabel("Batch size (points)")
    plt.ylabel("Throughput (points/s)")
    plt.title("DataPoint Creation Throughput")
    plt.xscale("log")
    plt.yscale("log")
    plt.grid(True, which="both", ls=":")
    plt.legend()
    plt.tight_layout()
    plt.savefig(path / "throughput.png", dpi=180)
    plt.close()


# ---------------------------------------------------------------------------
# Entrypoint
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=20_000, help="Micro benchmark sample count")
    parser.add_argument("--repeats", type=int, default=5, help="Repeats for throughput benchmark")
    parser.add_argument("--runs", type=int, default=3, help="Runs for memory benchmark")
    parser.add_argument("--rate", type=int, default=1_000, help="Acquisition rate in Hz for pipeline benchmark")
    parser.add_argument("--duration", type=int, default=10, help="Duration in seconds for pipeline benchmark")
    parser.add_argument("--json", type=Path, help="Optional path to write JSON report")
    parser.add_argument("--plot-dir", type=Path, help="Optional directory for charts (requires matplotlib)")
    args = parser.parse_args()

    creation = run_creation_bench(args.samples)
    throughput = run_throughput_bench((1_000, 10_000, 100_000), args.repeats)
    memory = run_memory_bench((1_000, 10_000, 100_000), args.runs)
    pipeline = run_pipeline_bench(args.rate, args.duration)

    maybe_write_json(args.json, {
        "creation": creation,
        "throughput": throughput,
        "memory": memory,
        "pipeline": pipeline,
    })
    maybe_plot_throughput(args.plot_dir, throughput)

    print("=== DataPoint Creation ===")
    print("variant | ops/sec | avg_us | p50_us | p95_us | p99_us")
    for label, metrics in creation.items():
        print(
            f"{label:<15} | {metrics['ops_per_sec']:>9.0f} | {metrics['avg_us']:>7.3f} | "
            f"{metrics['p50_us']:>7.3f} | {metrics['p95_us']:>7.3f} | {metrics['p99_us']:>7.3f}"
        )

    print("\n=== Throughput (points/s) ===")
    print("size | variant | throughput | median_s | avg_s | stdev_s")
    for label, series in throughput.items():
        for size, metrics in series.items():
            print(
                f"{size:>5} | {label:<15} | {metrics['throughput_pts_per_s']:>10.0f} | "
                f"{metrics['median_s']:>7.4f} | {metrics['avg_s']:>7.4f} | {metrics['stdev_s']:>7.4f}"
            )

    print("\n=== Memory (bytes) ===")
    print("size | variant | current | peak | avg_current | avg_peak")
    for label, series in memory.items():
        for size, metrics in series.items():
            print(
                f"{size:>5} | {label:<15} | {metrics['current_bytes']:>7.0f} | {metrics['peak_bytes']:>7.0f} | "
                f"{metrics['avg_current_bytes']:>11.0f} | {metrics['avg_peak_bytes']:>9.0f}"
            )

    print("\n=== 1 kHz Pipeline ===")
    print("variant | creation_s | pipeline_s | total_s | effective_rate")
    for label, metrics in pipeline.items():
        print(
            f"{label:<15} | {metrics['creation_time_s']:>10.4f} | {metrics['pipeline_time_s']:>10.4f} | "
            f"{metrics['total_time_s']:>8.4f} | {metrics['effective_rate']:>14.0f}"
        )


if __name__ == "__main__":
    main()
