#!/usr/bin/env python3
"""Analyze echelle extraction soak test results.

Reads the CSV output from overnight-soak.sh and generates:
  - Memory usage over time plot
  - Frame rate over time plot
  - Latency distribution histogram
  - Summary statistics table
  - PASS/FAIL determination

Usage:
    python3 analyze-soak-results.py /tmp/echelle-soak-YYYYMMDD-HHMMSS
    python3 analyze-soak-results.py /tmp/echelle-soak-YYYYMMDD-HHMMSS --threshold-mem-kb 1024
    python3 analyze-soak-results.py --help
"""

from __future__ import annotations

import argparse
import csv
import math
import os
import sys
from dataclasses import dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------
# Thresholds
# ---------------------------------------------------------------------------
DEFAULT_THRESHOLD_MEM_KB_PER_HOUR = 1024  # 1 MB/hour
DEFAULT_THRESHOLD_FRAME_DROPS = 0
DEFAULT_THRESHOLD_P99_LATENCY_MS = 500.0
DEFAULT_THRESHOLD_SIDECAR_CRASHES = 0


@dataclass
class SoakSample:
    """Single row from the soak CSV."""

    timestamp: str = ""
    elapsed_s: float = 0.0
    rss_kb: float | None = None
    frame_number: int | None = None
    frames_sent: int | None = None
    frames_dropped: int | None = None
    avg_latency_ms: float | None = None
    sidecar_alive: str = ""
    load_avg: float | None = None
    daemon_healthy: int = 1


@dataclass
class SoakAnalysis:
    """Aggregated soak test analysis."""

    samples: list[SoakSample] = field(default_factory=list)
    duration_hours: float = 0.0
    mem_growth_kb_per_hour: float | None = None
    total_frame_drops: int = 0
    total_frames: int = 0
    sidecar_crashes: int = 0
    daemon_health_losses: int = 0
    latency_p50: float | None = None
    latency_p99: float | None = None
    latencies: list[float] = field(default_factory=list)
    rss_values: list[float] = field(default_factory=list)
    rss_times_h: list[float] = field(default_factory=list)
    frame_rates: list[float] = field(default_factory=list)
    frame_rate_times_h: list[float] = field(default_factory=list)


def parse_float(s: str) -> float | None:
    """Parse a string as float, returning None on failure."""
    if not s or s.strip() == "":
        return None
    try:
        return float(s)
    except ValueError:
        return None


def parse_int(s: str) -> int | None:
    """Parse a string as int, returning None on failure."""
    if not s or s.strip() == "":
        return None
    try:
        return int(s)
    except ValueError:
        return None


def load_csv(csv_path: Path) -> list[SoakSample]:
    """Load soak CSV into a list of SoakSample objects."""
    samples: list[SoakSample] = []
    with open(csv_path, newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            sample = SoakSample(
                timestamp=row.get("timestamp", ""),
                elapsed_s=float(row.get("elapsed_s", 0)),
                rss_kb=parse_float(row.get("rss_kb", "")),
                frame_number=parse_int(row.get("frame_number", "")),
                frames_sent=parse_int(row.get("frames_sent", "")),
                frames_dropped=parse_int(row.get("frames_dropped", "")),
                avg_latency_ms=parse_float(row.get("avg_latency_ms", "")),
                sidecar_alive=row.get("sidecar_alive", ""),
                load_avg=parse_float(row.get("load_avg", "")),
                daemon_healthy=int(row.get("daemon_healthy", 1)),
            )
            samples.append(sample)
    return samples


def linreg_slope(xs: list[float], ys: list[float]) -> float:
    """Simple linear regression slope."""
    n = len(xs)
    if n < 2:
        return 0.0
    sx = sum(xs)
    sy = sum(ys)
    sxy = sum(x * y for x, y in zip(xs, ys))
    sxx = sum(x * x for x in xs)
    denom = n * sxx - sx * sx
    if abs(denom) < 1e-12:
        return 0.0
    return (n * sxy - sx * sy) / denom


def percentile(values: list[float], p: float) -> float:
    """Compute the p-th percentile (0-100) of a sorted list."""
    if not values:
        return 0.0
    s = sorted(values)
    n = len(s)
    idx = p / 100.0 * (n - 1)
    lo = int(idx)
    hi = min(lo + 1, n - 1)
    frac = idx - lo
    return s[lo] + frac * (s[hi] - s[lo])


def analyze(samples: list[SoakSample]) -> SoakAnalysis:
    """Compute aggregate analysis from samples."""
    analysis = SoakAnalysis(samples=samples)

    if not samples:
        return analysis

    # Duration
    max_elapsed = max(s.elapsed_s for s in samples)
    analysis.duration_hours = max_elapsed / 3600.0

    # RSS data for memory growth
    rss_times_s: list[float] = []
    rss_vals: list[float] = []
    for s in samples:
        if s.rss_kb is not None:
            rss_times_s.append(s.elapsed_s)
            rss_vals.append(s.rss_kb)
            analysis.rss_values.append(s.rss_kb)
            analysis.rss_times_h.append(s.elapsed_s / 3600.0)

    if len(rss_times_s) >= 2:
        slope_per_sec = linreg_slope(rss_times_s, rss_vals)
        analysis.mem_growth_kb_per_hour = slope_per_sec * 3600.0

    # Frame drops
    drops_values = [s.frames_dropped for s in samples if s.frames_dropped is not None]
    if drops_values:
        analysis.total_frame_drops = max(drops_values)

    # Total frames
    frame_numbers = [s.frame_number for s in samples if s.frame_number is not None]
    if len(frame_numbers) >= 2:
        analysis.total_frames = frame_numbers[-1] - frame_numbers[0]

    # Frame rate (frames between consecutive samples / time delta)
    for i in range(1, len(samples)):
        prev = samples[i - 1]
        curr = samples[i]
        if (
            prev.frame_number is not None
            and curr.frame_number is not None
            and curr.elapsed_s > prev.elapsed_s
        ):
            dt = curr.elapsed_s - prev.elapsed_s
            df = curr.frame_number - prev.frame_number
            if dt > 0:
                fps = df / dt
                analysis.frame_rates.append(fps)
                analysis.frame_rate_times_h.append(curr.elapsed_s / 3600.0)

    # Latency
    analysis.latencies = [
        s.avg_latency_ms for s in samples if s.avg_latency_ms is not None
    ]
    if analysis.latencies:
        analysis.latency_p50 = percentile(analysis.latencies, 50)
        analysis.latency_p99 = percentile(analysis.latencies, 99)

    # Sidecar crashes (transitions from alive to not-alive)
    prev_alive = None
    for s in samples:
        alive = s.sidecar_alive
        if prev_alive == "1" and alive == "0":
            analysis.sidecar_crashes += 1
        prev_alive = alive

    # Daemon health losses
    analysis.daemon_health_losses = sum(1 for s in samples if s.daemon_healthy == 0)

    return analysis


def generate_plots(analysis: SoakAnalysis, output_dir: Path) -> list[Path]:
    """Generate matplotlib plots. Returns list of created PNG paths.

    If matplotlib is not available, prints a warning and returns empty list.
    """
    try:
        import matplotlib

        matplotlib.use("Agg")  # Non-interactive backend
        import matplotlib.pyplot as plt
    except ImportError:
        print(
            "WARNING: matplotlib not available; skipping plot generation.",
            file=sys.stderr,
        )
        print(
            "Install with: pip install matplotlib",
            file=sys.stderr,
        )
        return []

    plots: list[Path] = []

    # 1) Memory usage over time
    if analysis.rss_times_h and analysis.rss_values:
        fig, ax = plt.subplots(figsize=(12, 5))
        ax.plot(analysis.rss_times_h, [v / 1024.0 for v in analysis.rss_values], "b-", linewidth=0.8)
        ax.set_xlabel("Elapsed (hours)")
        ax.set_ylabel("RSS (MB)")
        ax.set_title("Daemon Memory Usage Over Time")
        ax.grid(True, alpha=0.3)

        # Add regression line
        if analysis.mem_growth_kb_per_hour is not None and len(analysis.rss_times_h) >= 2:
            import numpy as np

            try:
                xs = np.array(analysis.rss_times_h)
                slope_mb_per_hour = analysis.mem_growth_kb_per_hour / 1024.0
                intercept = np.mean([v / 1024.0 for v in analysis.rss_values]) - slope_mb_per_hour * np.mean(xs)
                ax.plot(xs, slope_mb_per_hour * xs + intercept, "r--", linewidth=1.0,
                        label=f"Trend: {analysis.mem_growth_kb_per_hour:.1f} KB/h ({slope_mb_per_hour:.3f} MB/h)")
                ax.legend()
            except ImportError:
                pass

        path = output_dir / "memory_usage.png"
        fig.savefig(path, dpi=150, bbox_inches="tight")
        plt.close(fig)
        plots.append(path)
        print(f"  Plot: {path}")

    # 2) Frame rate over time
    if analysis.frame_rate_times_h and analysis.frame_rates:
        fig, ax = plt.subplots(figsize=(12, 5))
        ax.plot(analysis.frame_rate_times_h, analysis.frame_rates, "g-", linewidth=0.8)
        ax.set_xlabel("Elapsed (hours)")
        ax.set_ylabel("Frame Rate (fps)")
        ax.set_title("Frame Rate Over Time")
        ax.grid(True, alpha=0.3)
        if analysis.frame_rates:
            mean_fps = sum(analysis.frame_rates) / len(analysis.frame_rates)
            ax.axhline(y=mean_fps, color="orange", linestyle="--", linewidth=1.0,
                        label=f"Mean: {mean_fps:.1f} fps")
            ax.legend()
        path = output_dir / "frame_rate.png"
        fig.savefig(path, dpi=150, bbox_inches="tight")
        plt.close(fig)
        plots.append(path)
        print(f"  Plot: {path}")

    # 3) Latency distribution histogram
    if analysis.latencies:
        fig, ax = plt.subplots(figsize=(10, 5))
        n_bins = min(50, max(10, len(analysis.latencies) // 5))
        ax.hist(analysis.latencies, bins=n_bins, color="steelblue", edgecolor="white", alpha=0.8)
        ax.set_xlabel("Avg Latency (ms)")
        ax.set_ylabel("Count")
        ax.set_title("Extraction Latency Distribution")
        if analysis.latency_p50 is not None:
            ax.axvline(x=analysis.latency_p50, color="orange", linestyle="--",
                        label=f"P50: {analysis.latency_p50:.1f} ms")
        if analysis.latency_p99 is not None:
            ax.axvline(x=analysis.latency_p99, color="red", linestyle="--",
                        label=f"P99: {analysis.latency_p99:.1f} ms")
        ax.legend()
        ax.grid(True, alpha=0.3)
        path = output_dir / "latency_distribution.png"
        fig.savefig(path, dpi=150, bbox_inches="tight")
        plt.close(fig)
        plots.append(path)
        print(f"  Plot: {path}")

    return plots


def print_summary(
    analysis: SoakAnalysis,
    thresholds: dict[str, float],
    output_dir: Path,
) -> bool:
    """Print summary statistics and PASS/FAIL. Returns True if all checks pass."""
    all_pass = True

    print()
    print("=" * 60)
    print("Echelle Extraction Soak Test Analysis")
    print("=" * 60)
    print(f"Duration:              {analysis.duration_hours:.2f} hours")
    print(f"Samples:               {len(analysis.samples)}")
    print()

    # Memory growth
    mem_thresh = thresholds["mem_kb_per_hour"]
    if analysis.mem_growth_kb_per_hour is not None:
        abs_growth = abs(analysis.mem_growth_kb_per_hour)
        status = "PASS" if abs_growth <= mem_thresh else "FAIL"
        if status == "FAIL":
            all_pass = False
        print(f"Memory growth:         {analysis.mem_growth_kb_per_hour:+.1f} KB/hour "
              f"(threshold: {mem_thresh:.0f} KB/hour) [{status}]")
    else:
        print("Memory growth:         N/A (no RSS data)")

    # Frame drops
    drop_thresh = int(thresholds["frame_drops"])
    status = "PASS" if analysis.total_frame_drops <= drop_thresh else "FAIL"
    if status == "FAIL":
        all_pass = False
    print(f"Frame drops:           {analysis.total_frame_drops} "
          f"(threshold: {drop_thresh}) [{status}]")

    # Total frames
    print(f"Total frames:          {analysis.total_frames}")

    # Sidecar crashes
    sc_thresh = int(thresholds["sidecar_crashes"])
    status = "PASS" if analysis.sidecar_crashes <= sc_thresh else "FAIL"
    if status == "FAIL":
        all_pass = False
    print(f"Sidecar crashes:       {analysis.sidecar_crashes} "
          f"(threshold: {sc_thresh}) [{status}]")

    # Daemon health
    status = "PASS" if analysis.daemon_health_losses == 0 else "FAIL"
    if status == "FAIL":
        all_pass = False
    print(f"Daemon health losses:  {analysis.daemon_health_losses} [{status}]")

    # Latency
    lat_thresh = thresholds["p99_latency_ms"]
    if analysis.latency_p50 is not None and analysis.latency_p99 is not None:
        status = "PASS" if analysis.latency_p99 <= lat_thresh else "FAIL"
        if status == "FAIL":
            all_pass = False
        print(f"Latency P50:           {analysis.latency_p50:.1f} ms")
        print(f"Latency P99:           {analysis.latency_p99:.1f} ms "
              f"(threshold: {lat_thresh:.0f} ms) [{status}]")
    else:
        print("Latency:               N/A (no latency data)")

    # Frame rate stats
    if analysis.frame_rates:
        mean_fps = sum(analysis.frame_rates) / len(analysis.frame_rates)
        min_fps = min(analysis.frame_rates)
        max_fps = max(analysis.frame_rates)
        print(f"Frame rate:            mean={mean_fps:.1f} min={min_fps:.1f} max={max_fps:.1f} fps")

    # RSS stats
    if analysis.rss_values:
        min_rss = min(analysis.rss_values)
        max_rss = max(analysis.rss_values)
        mean_rss = sum(analysis.rss_values) / len(analysis.rss_values)
        print(f"RSS:                   mean={mean_rss/1024:.1f}MB min={min_rss/1024:.1f}MB max={max_rss/1024:.1f}MB")

    result = "PASS" if all_pass else "FAIL"
    print()
    print(f"RESULT: {result}")
    print("=" * 60)

    # Write summary to file
    summary_path = output_dir / "analysis_summary.txt"
    with open(summary_path, "w") as f:
        f.write(f"Result: {result}\n")
        f.write(f"Duration: {analysis.duration_hours:.2f} hours\n")
        f.write(f"Samples: {len(analysis.samples)}\n")
        if analysis.mem_growth_kb_per_hour is not None:
            f.write(f"Memory growth: {analysis.mem_growth_kb_per_hour:+.1f} KB/hour\n")
        f.write(f"Frame drops: {analysis.total_frame_drops}\n")
        f.write(f"Total frames: {analysis.total_frames}\n")
        f.write(f"Sidecar crashes: {analysis.sidecar_crashes}\n")
        f.write(f"Daemon health losses: {analysis.daemon_health_losses}\n")
        if analysis.latency_p50 is not None:
            f.write(f"Latency P50: {analysis.latency_p50:.1f} ms\n")
        if analysis.latency_p99 is not None:
            f.write(f"Latency P99: {analysis.latency_p99:.1f} ms\n")
    print(f"\nSummary written to: {summary_path}")

    return all_pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Analyze echelle extraction soak test results",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "output_dir",
        type=Path,
        help="Soak test output directory (containing samples.csv)",
    )
    parser.add_argument(
        "--csv",
        type=Path,
        default=None,
        help="Path to CSV file (default: <output_dir>/samples.csv)",
    )
    parser.add_argument(
        "--threshold-mem-kb",
        type=float,
        default=DEFAULT_THRESHOLD_MEM_KB_PER_HOUR,
        help=f"Max memory growth KB/hour (default: {DEFAULT_THRESHOLD_MEM_KB_PER_HOUR})",
    )
    parser.add_argument(
        "--threshold-frame-drops",
        type=int,
        default=DEFAULT_THRESHOLD_FRAME_DROPS,
        help=f"Max allowed frame drops (default: {DEFAULT_THRESHOLD_FRAME_DROPS})",
    )
    parser.add_argument(
        "--threshold-p99-ms",
        type=float,
        default=DEFAULT_THRESHOLD_P99_LATENCY_MS,
        help=f"Max P99 extraction latency ms (default: {DEFAULT_THRESHOLD_P99_LATENCY_MS})",
    )
    parser.add_argument(
        "--threshold-sidecar-crashes",
        type=int,
        default=DEFAULT_THRESHOLD_SIDECAR_CRASHES,
        help=f"Max allowed sidecar crashes (default: {DEFAULT_THRESHOLD_SIDECAR_CRASHES})",
    )
    parser.add_argument(
        "--no-plots",
        action="store_true",
        help="Skip plot generation (useful in headless environments without matplotlib)",
    )

    args = parser.parse_args()

    output_dir: Path = args.output_dir
    csv_path: Path = args.csv if args.csv else output_dir / "samples.csv"

    if not csv_path.exists():
        print(f"ERROR: CSV file not found: {csv_path}", file=sys.stderr)
        return 1

    if not output_dir.exists():
        print(f"ERROR: Output directory not found: {output_dir}", file=sys.stderr)
        return 1

    print(f"Loading CSV: {csv_path}")
    samples = load_csv(csv_path)

    if not samples:
        print("ERROR: No samples found in CSV", file=sys.stderr)
        return 1

    print(f"Loaded {len(samples)} samples")

    analysis = analyze(samples)

    # Generate plots
    if not args.no_plots:
        print("\nGenerating plots...")
        plots = generate_plots(analysis, output_dir)
        if plots:
            print(f"  {len(plots)} plot(s) saved to {output_dir}")

    # Print summary and determine PASS/FAIL
    thresholds = {
        "mem_kb_per_hour": args.threshold_mem_kb,
        "frame_drops": args.threshold_frame_drops,
        "p99_latency_ms": args.threshold_p99_ms,
        "sidecar_crashes": args.threshold_sidecar_crashes,
    }
    all_pass = print_summary(analysis, thresholds, output_dir)

    return 0 if all_pass else 1


if __name__ == "__main__":
    sys.exit(main())
