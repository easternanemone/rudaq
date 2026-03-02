#!/usr/bin/env python3
"""
Fixture-backed prototype echelle sidecar for offline/captured frame testing.

This is not a production extractor. It serves committed reference artifacts for
the leabs-dev HG-2 dataset so rust-daq can exercise the JSON-over-stdio sidecar
path and compare numeric outputs/provenance against the local Rust MVP path.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path


DEFAULT_DATASET_REL = Path("testdata/echelle/leabs-dev/2026-02-25-hg2")


def emit_stderr(event: dict) -> None:
    sys.stderr.write(json.dumps(event) + "\n")
    sys.stderr.flush()


def read_json(path: Path) -> dict:
    return json.loads(path.read_text())


def health_response(request_id: str, dataset_dir: Path) -> dict:
    return {
        "request_id": request_id,
        "ok": True,
        "result": {
            "service": "fixture_sidecar_hg2",
            "version": "0.1",
            "dataset_dir": str(dataset_dir),
            "ops": ["health", "extract_offline_fixture", "extract_offline"],
        },
    }


def offline_fixture_response(request_id: str, dataset_dir: Path, label: str) -> dict:
    ref_dir = dataset_dir / "reference"
    frame_summary = read_json(dataset_dir / f"{label}_summary.json")
    ref_summary = read_json(ref_dir / f"{label}_reference_summary.json")
    tolerances = read_json(ref_dir / "comparison_tolerances.json")
    capture_diag = read_json(ref_dir / "capture_diagnostics.json")
    provenance = read_json(ref_dir / "provenance.json")

    capture_diag_entry = capture_diag["captures"].get(label, {})
    pairwise = [
        item
        for item in capture_diag.get("pairwise_differences", [])
        if item.get("a") == label or item.get("b") == label
    ]

    return {
        "request_id": request_id,
        "ok": True,
        "result": {
            "orders": [],
            "merged": None,
            "quality": {
                "dataset_classification": tolerances.get("dataset_classification"),
                "spectroscopic_truth_usable": tolerances.get("spectroscopic_truth_usable"),
                "reference_order_count": ref_summary.get("order_detection", {}).get("n_orders"),
                "reference_quality_kind": ref_summary.get("dataset_quality_classification", {}).get(
                    "kind"
                ),
                "capture_image_diagnostics": capture_diag_entry,
                "pairwise_differences_involving_capture": pairwise,
            },
            "provenance": {
                "sidecar_name": "fixture_sidecar_hg2",
                "sidecar_kind": "fixture_reference_json",
                "dataset_id": tolerances.get("dataset_id"),
                "label": label,
                "reference_generator": provenance.get("tool", {}),
            },
            "frame": frame_summary.get("frame", {}),
            "reference_summary": ref_summary,
            "tolerances": tolerances,
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dataset-dir", type=Path, default=DEFAULT_DATASET_REL)
    args = ap.parse_args()
    dataset_dir = args.dataset_dir.resolve()

    start = time.time()
    line = sys.stdin.readline()
    if not line:
        emit_stderr({"level": "error", "msg": "empty stdin request"})
        return 1

    try:
        req = json.loads(line)
    except json.JSONDecodeError as e:
        emit_stderr({"level": "error", "msg": "invalid request json", "error": str(e)})
        print(
            json.dumps(
                {
                    "request_id": None,
                    "ok": False,
                    "error": {"code": "INVALID_REQUEST", "message": str(e)},
                }
            )
        )
        return 0

    request_id = req.get("request_id")
    op = req.get("op")
    emit_stderr(
        {
            "level": "info",
            "msg": "request_received",
            "op": op,
            "request_id": request_id,
            "dataset_dir": str(dataset_dir),
        }
    )

    if op == "health":
        resp = health_response(request_id, dataset_dir)
    elif op in ("extract_offline_fixture", "extract_offline"):
        label = req.get("label")
        if not isinstance(label, str) or not label:
            resp = {
                "request_id": request_id,
                "ok": False,
                "error": {"code": "INVALID_REQUEST", "message": "label is required"},
            }
        else:
            resp = offline_fixture_response(request_id, dataset_dir, label)
    else:
        resp = {
            "request_id": request_id,
            "ok": False,
            "error": {"code": "INVALID_REQUEST", "message": f"unsupported op: {op!r}"},
        }

    emit_stderr(
        {
            "level": "info",
            "msg": "request_complete",
            "request_id": request_id,
            "ok": resp.get("ok"),
            "elapsed_ms": round((time.time() - start) * 1000.0, 3),
        }
    )
    print(json.dumps(resp))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
