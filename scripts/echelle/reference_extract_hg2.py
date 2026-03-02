#!/usr/bin/env python3
"""
Generate pixel-space reference ("golden") echelle extractions for the committed
leabs-dev HG-2 / Mechelle / iSTAR capture set.

This is intentionally external to rust-daq's UI extractor implementation and is
used to create comparison artifacts plus provenance.
"""

from __future__ import annotations

import argparse
import csv
import gzip
import hashlib
import json
import os
import sys
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Iterable


def _prepend_pydeps() -> None:
    root = Path(__file__).resolve().parents[2]
    dep_dir = root / ".pydeps-echelle-ref"
    if dep_dir.exists():
        sys.path.insert(0, str(dep_dir))


_prepend_pydeps()

import numpy as np  # type: ignore
import lz4.block  # type: ignore


DATASET_REL = Path("testdata/echelle/leabs-dev/2026-02-25-hg2")


@dataclass
class DetectionConfig:
    smooth_window: int = 31
    min_peak_distance_px: int = 18
    min_peak_prominence: float = 0.10
    max_orders: int = 24
    aperture_half_width_px: int = 2
    bg_gap_px: int = 4
    bg_width_px: int = 3


@dataclass
class ImageDiagnostics:
    min: int
    max: int
    mean: float
    stddev: float
    row_profile_stddev: float
    col_profile_stddev: float
    residual_stddev: float
    residual_to_image_stddev_ratio: float
    sampled_additive_ramp_match: bool
    sampled_additive_ramp_max_abs_error: float
    likely_additive_ramp_pattern: bool


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def decompress_lz4_size_prepended(payload: bytes) -> bytes:
    if len(payload) < 4:
        raise ValueError("payload too small for size-prepended lz4")
    expected = int.from_bytes(payload[:4], "little")
    raw = lz4.block.decompress(payload[4:], uncompressed_size=expected)
    if len(raw) != expected:
        raise ValueError(f"decompressed size mismatch: got {len(raw)}, expected {expected}")
    return raw


def moving_average(a: np.ndarray, window: int) -> np.ndarray:
    if window <= 1:
        return a.astype(np.float64, copy=True)
    window = int(max(1, window))
    kernel = np.ones(window, dtype=np.float64) / float(window)
    return np.convolve(a.astype(np.float64), kernel, mode="same")


def estimate_separable_background(image_u16: np.ndarray) -> np.ndarray:
    image = image_u16.astype(np.float64, copy=False)
    row_mean = image.mean(axis=1, dtype=np.float64)
    col_mean = image.mean(axis=0, dtype=np.float64)
    global_mean = float(image.mean(dtype=np.float64))
    return row_mean[:, None] + col_mean[None, :] - global_mean


def compute_image_diagnostics(image_u16: np.ndarray) -> tuple[ImageDiagnostics, np.ndarray]:
    image = image_u16.astype(np.float64, copy=False)
    sep_bg = estimate_separable_background(image_u16)
    residual = image - sep_bg

    # Sample whether image ~= row_index + col_index (the observed diagnostic ramp pattern)
    h, w = image_u16.shape
    rs = np.linspace(0, h - 1, num=min(32, h), dtype=np.int32)
    cs = np.linspace(0, w - 1, num=min(32, w), dtype=np.int32)
    sampled_errors = []
    for r in rs.tolist():
        for c in cs.tolist():
            sampled_errors.append(float(image_u16[r, c]) - float(r + c))
    sampled_errors_arr = np.array(sampled_errors, dtype=np.float64)
    sampled_additive_ramp_max_abs_error = float(np.max(np.abs(sampled_errors_arr))) if sampled_errors else 0.0
    sampled_additive_ramp_match = bool(sampled_additive_ramp_max_abs_error <= 1.0)

    stddev = float(image.std(dtype=np.float64))
    residual_stddev = float(residual.std(dtype=np.float64))
    residual_ratio = residual_stddev / stddev if stddev > 0 else 0.0

    diag = ImageDiagnostics(
        min=int(image_u16.min()),
        max=int(image_u16.max()),
        mean=float(image.mean(dtype=np.float64)),
        stddev=stddev,
        row_profile_stddev=float(image.sum(axis=1, dtype=np.float64).std(dtype=np.float64)),
        col_profile_stddev=float(image.sum(axis=0, dtype=np.float64).std(dtype=np.float64)),
        residual_stddev=residual_stddev,
        residual_to_image_stddev_ratio=float(residual_ratio),
        sampled_additive_ramp_match=sampled_additive_ramp_match,
        sampled_additive_ramp_max_abs_error=sampled_additive_ramp_max_abs_error,
        likely_additive_ramp_pattern=bool(sampled_additive_ramp_match and residual_ratio < 1e-6),
    )
    return diag, residual


def detect_order_centers(image_u16: np.ndarray, cfg: DetectionConfig) -> tuple[np.ndarray, np.ndarray]:
    # Detrend separable row/column background so monotonic ramps do not trigger false "orders".
    _, residual = compute_image_diagnostics(image_u16)
    signal = np.clip(residual, 0.0, None)
    row_profile = signal.sum(axis=1, dtype=np.float64)
    smoothed = moving_average(row_profile, cfg.smooth_window)
    if smoothed.max() <= 0:
        return np.array([], dtype=np.int32), smoothed

    baseline = np.percentile(smoothed, 20)
    peak_min = baseline + cfg.min_peak_prominence * (smoothed.max() - baseline)

    candidates: list[int] = []
    for i in range(1, len(smoothed) - 1):
        if smoothed[i] >= peak_min and smoothed[i] >= smoothed[i - 1] and smoothed[i] > smoothed[i + 1]:
            candidates.append(i)

    # greedy select by amplitude with min separation
    candidates.sort(key=lambda i: smoothed[i], reverse=True)
    selected: list[int] = []
    for idx in candidates:
        if all(abs(idx - s) >= cfg.min_peak_distance_px for s in selected):
            selected.append(idx)
            if len(selected) >= cfg.max_orders:
                break
    selected.sort()
    return np.array(selected, dtype=np.int32), smoothed


def extract_orders(
    image_u16: np.ndarray,
    order_centers: np.ndarray,
    cfg: DetectionConfig,
) -> dict[str, np.ndarray]:
    h, w = image_u16.shape
    x = np.arange(w, dtype=np.int32)
    n_orders = int(order_centers.shape[0])
    flux_raw = np.zeros((n_orders, w), dtype=np.float64)
    flux_bgsub = np.zeros((n_orders, w), dtype=np.float64)
    valid_fraction = np.zeros((n_orders, w), dtype=np.float32)
    sat_any = np.zeros((n_orders, w), dtype=np.bool_)

    ap = cfg.aperture_half_width_px
    bg_gap = cfg.bg_gap_px
    bg_w = cfg.bg_width_px

    for oi, cy in enumerate(order_centers.tolist()):
        rows = np.arange(cy - ap, cy + ap + 1, dtype=np.int32)
        rows_clip = rows[(rows >= 0) & (rows < h)]
        if rows_clip.size == 0:
            continue
        sub = image_u16[rows_clip, :]
        raw = sub.sum(axis=0, dtype=np.float64)
        flux_raw[oi] = raw
        valid_fraction[oi] = rows_clip.size / rows.size
        sat_any[oi] = (sub >= 4095).any(axis=0)

        # Symmetric sideband mean subtraction
        bg_rows = np.concatenate(
            [
                np.arange(cy - ap - bg_gap - bg_w + 1, cy - ap - bg_gap + 1, dtype=np.int32),
                np.arange(cy + ap + bg_gap, cy + ap + bg_gap + bg_w, dtype=np.int32),
            ]
        )
        bg_clip = bg_rows[(bg_rows >= 0) & (bg_rows < h)]
        if bg_clip.size > 0:
            bg_mean = image_u16[bg_clip, :].mean(axis=0, dtype=np.float64)
            flux_bgsub[oi] = raw - bg_mean * rows_clip.size
        else:
            flux_bgsub[oi] = raw

    return {
        "x": x,
        "flux_raw": flux_raw,
        "flux_bgsub": flux_bgsub,
        "valid_fraction": valid_fraction,
        "saturation_any": sat_any,
    }


def write_orders_csv_gz(
    out_path: Path,
    centers: np.ndarray,
    arrays: dict[str, np.ndarray],
) -> None:
    x = arrays["x"]
    flux_raw = arrays["flux_raw"]
    flux_bgsub = arrays["flux_bgsub"]
    valid_fraction = arrays["valid_fraction"]
    saturation_any = arrays["saturation_any"]
    with gzip.open(out_path, "wt", newline="") as f:
        w = csv.writer(f)
        w.writerow(
            ["order_index", "center_row", "sample_x", "flux_raw", "flux_bgsub", "valid_fraction", "saturated"]
        )
        for oi, cy in enumerate(centers.tolist()):
            for xi in range(x.shape[0]):
                w.writerow(
                    [
                        oi,
                        int(cy),
                        int(x[xi]),
                        f"{float(flux_raw[oi, xi]):.6f}",
                        f"{float(flux_bgsub[oi, xi]):.6f}",
                        f"{float(valid_fraction[oi, xi]):.6f}",
                        int(bool(saturation_any[oi, xi])),
                    ]
                )


def summarize_capture(
    label: str,
    frame_summary: dict,
    centers: np.ndarray,
    arrays: dict[str, np.ndarray],
    smoothed_row_profile: np.ndarray,
    diagnostics: ImageDiagnostics,
    cfg: DetectionConfig,
) -> dict:
    flux_bgsub = arrays["flux_bgsub"]
    sat_any = arrays["saturation_any"]
    per_order = []
    for oi, cy in enumerate(centers.tolist()):
        spectrum = flux_bgsub[oi]
        per_order.append(
            {
                "order_index": oi,
                "center_row": int(cy),
                "peak_flux_bgsub": float(np.max(spectrum)),
                "mean_flux_bgsub": float(np.mean(spectrum)),
                "saturated_samples": int(np.count_nonzero(sat_any[oi])),
            }
        )
    return {
        "label": label,
        "frame_summary": frame_summary,
        "reference_extractor": {
            "axis": "pixel_x",
            "wavelength_calibrated": False,
            "algorithm": "row-profile peak detect + constant-row top-hat extraction + optional sideband background",
            "config": asdict(cfg),
        },
        "order_detection": {
            "n_orders": int(centers.shape[0]),
            "center_rows": [int(x) for x in centers.tolist()],
            "row_profile_max": float(np.max(smoothed_row_profile)) if smoothed_row_profile.size else 0.0,
        },
        "image_diagnostics": asdict(diagnostics),
        "dataset_quality_classification": {
            "kind": "diagnostic_ramp_like" if diagnostics.likely_additive_ramp_pattern else "candidate_spectral_frame",
            "usable_for_spectroscopic_truth": False if diagnostics.likely_additive_ramp_pattern else True,
        },
        "per_order_summary": per_order,
    }


def parse_frame_and_payload(dataset_dir: Path, label: str) -> tuple[dict, np.ndarray]:
    summary = json.loads((dataset_dir / f"{label}_summary.json").read_text())
    frame_meta = summary["frame"]
    payload = (dataset_dir / f"{label}_payload_lz4.bin").read_bytes()
    raw = decompress_lz4_size_prepended(payload)
    width = int(frame_meta["width"])
    height = int(frame_meta["height"])
    bit_depth = int(frame_meta["bit_depth"])
    if bit_depth not in (12, 16):
        raise ValueError(f"unsupported bit depth for reference extractor: {bit_depth}")
    # 12-bit is transported as 16-bit little-endian words in this stream path.
    arr = np.frombuffer(raw, dtype="<u2").reshape(height, width)
    return summary, arr


def build_capture_diagnostics(labels: list[str], images: dict[str, np.ndarray]) -> dict:
    captures = {}
    for label in labels:
        diag, _ = compute_image_diagnostics(images[label])
        captures[label] = asdict(diag)

    pairwise = []
    for i in range(len(labels)):
        for j in range(i + 1, len(labels)):
            a_name = labels[i]
            b_name = labels[j]
            a = images[a_name].astype(np.int32, copy=False)
            b = images[b_name].astype(np.int32, copy=False)
            d = a - b
            pairwise.append(
                {
                    "a": a_name,
                    "b": b_name,
                    "max_abs_diff": int(np.max(np.abs(d))),
                    "mean_diff": float(d.mean(dtype=np.float64)),
                    "nonzero_diff_pixels": int(np.count_nonzero(d)),
                    "p99_abs_diff": float(np.percentile(np.abs(d), 99)),
                }
            )
    return {"captures": captures, "pairwise_differences": pairwise}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dataset-dir", type=Path, default=DATASET_REL)
    ap.add_argument("--out-dir", type=Path, default=None)
    args = ap.parse_args()

    dataset_dir = args.dataset_dir.resolve()
    out_dir = (args.out_dir or (dataset_dir / "reference")).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    cfg = DetectionConfig()
    labels = ["hg2_001ms", "hg2_010ms", "hg2_100ms"]
    capture_summaries = []
    capture_images: dict[str, np.ndarray] = {}

    for label in labels:
        summary, image = parse_frame_and_payload(dataset_dir, label)
        capture_images[label] = image
        diagnostics, _ = compute_image_diagnostics(image)
        centers, smoothed = detect_order_centers(image, cfg)
        arrays = extract_orders(image, centers, cfg)

        npz_path = out_dir / f"{label}_reference_orders_pixel.npz"
        np.savez_compressed(
            npz_path,
            order_centers=centers,
            smoothed_row_profile=smoothed,
            **arrays,
        )
        csv_gz_path = out_dir / f"{label}_reference_orders_pixel.csv.gz"
        write_orders_csv_gz(csv_gz_path, centers, arrays)

        capture_summary = summarize_capture(label, summary["frame"], centers, arrays, smoothed, diagnostics, cfg)
        (out_dir / f"{label}_reference_summary.json").write_text(json.dumps(capture_summary, indent=2))
        capture_summaries.append(capture_summary)

    # Provenance and dataset-level summary
    provenance = {
        "tool": {
            "name": "scripts/echelle/reference_extract_hg2.py",
            "kind": "external_reference_tooling",
            "python": sys.version,
            "numpy": np.__version__,
            "lz4": getattr(sys.modules.get("lz4"), "__version__", "unknown"),
        },
        "dataset_dir": str(dataset_dir),
        "source_manifest_sha256": sha256_file(dataset_dir / "manifest.json"),
        "source_parameters_sha256": sha256_file(dataset_dir / "parameters_raw.json"),
        "notes": [
            "Pixel-space reference extraction only (not wavelength calibrated).",
            "Order centers are estimated from row-profile peaks and treated as constant-row traces.",
            "Use for transport/decompression and regression baselines until calibrated traces/wavelength solutions are available.",
        ],
    }
    (out_dir / "provenance.json").write_text(json.dumps(provenance, indent=2))
    (out_dir / "dataset_reference_index.json").write_text(
        json.dumps({"captures": capture_summaries}, indent=2)
    )
    (out_dir / "capture_diagnostics.json").write_text(
        json.dumps(build_capture_diagnostics(labels, capture_images), indent=2)
    )

    print(f"Wrote reference outputs to {out_dir}")
    for p in sorted(out_dir.iterdir()):
        print(p.name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
