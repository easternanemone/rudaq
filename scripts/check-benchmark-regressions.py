#!/usr/bin/env python3
"""Run the benchmark regression gate used by CI."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
from json import JSONDecoder


DEFAULT_BASELINE = pathlib.Path(".github/performance-regression-baseline.json")
DEFAULT_OUTPUT_DIR = pathlib.Path("target/performance-gate")

RING_BUFFER_COMMAND = [
    "cargo",
    "test",
    "-p",
    "storage",
    "--lib",
    "bench_ring_buffer_write_throughput",
    "--release",
    "--",
    "--nocapture",
]

ECHELLE_COMMAND = [
    "cargo",
    "test",
    "-p",
    "ui",
    "--lib",
    "benchmark_real_canned_hg2_extraction_latency_and_live_budget",
    "--release",
    "--",
    "--ignored",
    "--nocapture",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline",
        type=pathlib.Path,
        default=DEFAULT_BASELINE,
        help="Path to the checked-in performance baseline JSON",
    )
    parser.add_argument(
        "--output-dir",
        type=pathlib.Path,
        default=DEFAULT_OUTPUT_DIR,
        help="Directory where raw logs, result JSON, and summary markdown are written",
    )
    return parser.parse_args()


def run_command(
    command: list[str], *, output_path: pathlib.Path, timeout_seconds: int
) -> str:
    result = subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
        timeout=timeout_seconds,
    )
    output_path.write_text(result.stdout, encoding="utf-8")
    if result.returncode != 0:
        print(result.stdout, file=sys.stderr, end="")
        raise SystemExit(
            f"benchmark command failed with exit code {result.returncode}: {' '.join(command)}"
        )
    return result.stdout


def parse_ring_buffer_output(output: str) -> dict[str, float]:
    fps_match = re.search(r"^fps:\s+([0-9.]+)$", output, flags=re.MULTILINE)
    throughput_match = re.search(
        r"^throughput:\s+([0-9.]+)\s+MB/s$", output, flags=re.MULTILINE
    )
    if fps_match is None or throughput_match is None:
        raise SystemExit("failed to parse ring buffer benchmark output")
    return {
        "fps": float(fps_match.group(1)),
        "throughput_mb_s": float(throughput_match.group(1)),
    }


def extract_first_json_object(output: str) -> dict:
    decoder = JSONDecoder()
    for index, character in enumerate(output):
        if character != "{":
            continue
        try:
            value, _end = decoder.raw_decode(output[index:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    raise SystemExit("failed to locate benchmark JSON in extraction output")


def load_baseline(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def evaluate_metric(
    metric_name: str, actual: float, config: dict, allowed_regression_pct: float
) -> dict:
    baseline = float(config["baseline"])
    direction = config["direction"]
    if direction == "higher_is_better":
        threshold = baseline * (1.0 - allowed_regression_pct / 100.0)
        passed = actual >= threshold
    elif direction == "lower_is_better":
        threshold = baseline * (1.0 + allowed_regression_pct / 100.0)
        passed = actual <= threshold
    else:
        raise SystemExit(f"unsupported direction for {metric_name}: {direction}")

    return {
        "metric": metric_name,
        "actual": actual,
        "baseline": baseline,
        "allowed_regression_pct": allowed_regression_pct,
        "direction": direction,
        "threshold": threshold,
        "unit": config["unit"],
        "passed": passed,
    }


def build_summary_lines(report: dict) -> list[str]:
    lines = [
        "# Performance Regression Gate",
        "",
        f"- Allowed regression: {report['allowed_regression_pct']:.1f}%",
        "",
        "| Benchmark | Metric | Baseline | Allowed Threshold | Actual | Status |",
        "|-----------|--------|----------|-------------------|--------|--------|",
    ]

    for benchmark_name, benchmark_report in report["benchmarks"].items():
        for metric in benchmark_report["metrics"]:
            baseline = f"{metric['baseline']:.3f} {metric['unit']}"
            threshold = f"{metric['threshold']:.3f} {metric['unit']}"
            actual = f"{metric['actual']:.3f} {metric['unit']}"
            status = "PASS" if metric["passed"] else "FAIL"
            lines.append(
                f"| `{benchmark_name}` | `{metric['metric']}` | {baseline} | {threshold} | {actual} | {status} |"
            )

    return lines


def append_github_summary(summary_path: pathlib.Path) -> None:
    os_path = os.environ.get("GITHUB_STEP_SUMMARY")
    github_summary = pathlib.Path(os_path) if os_path else None
    if github_summary is None:
        return
    github_summary.parent.mkdir(parents=True, exist_ok=True)
    with github_summary.open("a", encoding="utf-8") as handle:
        handle.write(summary_path.read_text(encoding="utf-8"))
        handle.write("\n")


def main() -> int:
    args = parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)

    baseline = load_baseline(args.baseline)
    allowed_regression_pct = float(baseline["allowed_regression_pct"])

    ring_output = run_command(
        RING_BUFFER_COMMAND,
        output_path=args.output_dir / "ring-buffer-write.log",
        timeout_seconds=300,
    )
    ring_metrics = parse_ring_buffer_output(ring_output)

    extraction_output = run_command(
        ECHELLE_COMMAND,
        output_path=args.output_dir / "echelle-extraction.log",
        timeout_seconds=600,
    )
    extraction_report = extract_first_json_object(extraction_output)

    expected_dataset = baseline["benchmarks"]["echelle_extraction_hg2"]["dataset_id"]
    actual_dataset = extraction_report.get("dataset_id")
    if actual_dataset != expected_dataset:
        raise SystemExit(
            f"unexpected extraction dataset id: expected {expected_dataset!r}, got {actual_dataset!r}"
        )

    report = {
        "schema_version": 1,
        "allowed_regression_pct": allowed_regression_pct,
        "benchmarks": {
            "ring_buffer_write_throughput": {
                "command": " ".join(RING_BUFFER_COMMAND),
                "metrics": [
                    evaluate_metric(
                        "fps",
                        ring_metrics["fps"],
                        baseline["benchmarks"]["ring_buffer_write_throughput"]["metrics"]["fps"],
                        allowed_regression_pct,
                    )
                ],
                "raw_metrics": ring_metrics,
            },
            "echelle_extraction_hg2": {
                "command": " ".join(ECHELLE_COMMAND),
                "dataset_id": actual_dataset,
                "metrics": [
                    evaluate_metric(
                        "throughput_frames_per_sec_decode_plus_extract",
                        float(
                            extraction_report[
                                "throughput_frames_per_sec_decode_plus_extract"
                            ]
                        ),
                        baseline["benchmarks"]["echelle_extraction_hg2"]["metrics"][
                            "throughput_frames_per_sec_decode_plus_extract"
                        ],
                        allowed_regression_pct,
                    ),
                    evaluate_metric(
                        "decode_plus_extract_p95_us",
                        float(extraction_report["decode_plus_extract_us"]["p95"]),
                        baseline["benchmarks"]["echelle_extraction_hg2"]["metrics"][
                            "decode_plus_extract_p95_us"
                        ],
                        allowed_regression_pct,
                    ),
                ],
                "raw_metrics": {
                    "throughput_frames_per_sec_decode_plus_extract": extraction_report[
                        "throughput_frames_per_sec_decode_plus_extract"
                    ],
                    "decode_plus_extract_us": extraction_report["decode_plus_extract_us"],
                    "live_budget_simulation": extraction_report["live_budget_simulation"],
                },
            },
        },
    }

    summary_lines = build_summary_lines(report)
    summary_path = args.output_dir / "summary.md"
    summary_path.write_text("\n".join(summary_lines) + "\n", encoding="utf-8")
    append_github_summary(summary_path)

    results_path = args.output_dir / "results.json"
    results_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    (args.output_dir / "extraction-report.json").write_text(
        json.dumps(extraction_report, indent=2, sort_keys=True), encoding="utf-8"
    )

    failures = [
        metric
        for benchmark_report in report["benchmarks"].values()
        for metric in benchmark_report["metrics"]
        if not metric["passed"]
    ]
    if failures:
        for metric in failures:
            print(
                "FAIL: "
                f"{metric['metric']} actual={metric['actual']:.3f}{metric['unit']} "
                f"threshold={metric['threshold']:.3f}{metric['unit']}",
                file=sys.stderr,
            )
        return 1

    print(summary_path.read_text(encoding="utf-8"), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
