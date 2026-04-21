#!/usr/bin/env python3
# /// script
# dependencies = ["numpy", "tifffile", "matplotlib"]
# ///
"""
bd-61aej — DH3P flat-field DoE analysis.

For each (MCP gain, exposure) frame, compute the metrics that tell us
whether the frame is usable as a flat-field reference for the Mechelle
5000 calibration pipeline:

  * saturation fraction — % of pixels at the detector ceiling. Flat-field
    trace detection tolerates partial saturation on the brightest orders
    (median-based smoothing still finds the centroid), but heavy
    saturation (>1%) means the dynamic range is compressed.
  * dynamic range — p99 / p50 on in-order pixels. Good flat-fields have
    ~10× dynamic range across the blaze envelope.
  * in-order / inter-order ratio — bright-order median divided by the
    10th-percentile (inter-order) pedestal. >5× is the rule of thumb
    for morphological scattered-light subtraction (the DH3P continuum
    must ride above the MCP halo to extract a clean flat).
  * faint-order SNR — median / std on the *faintest* order band, as a
    proxy for UV-end noise-floor coverage.

Outputs:
  * report.json — one entry per frame with the metrics above and a
    verdict tag (good | marginal | saturated | noise_floor).
  * plot.png    — 3×3 grid of small-multiples: log-scaled frame +
    overlaid saturation mask, cross-section at x=1280 (center column),
    and a dynamic-range bar per frame.

Usage:  uv run analyze_dh3p.py
"""

import json
from dataclasses import dataclass
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import tifffile

ROOT = Path(__file__).resolve().parent
MATRIX_JSON = ROOT / "matrix.json"
REPORT_JSON = ROOT / "report.json"
PLOT_PATH = ROOT / "plot.png"

# iStar sCMOS is 16-bit readout, but the MCP + digitisation often clip at
# 4095 (12-bit effective) on HgAr captures. For the continuum lamp the
# per-frame max tells us the effective ceiling. Use a relative threshold
# (≥98% of per-frame max) so we catch clipping at either bit depth.
SAT_RELATIVE_THRESHOLD = 0.98


@dataclass
class FrameMetrics:
    label: str
    mcp_gain: int
    exposure_ms: int
    accumulate_count: int
    effective_ms: int
    p50: float
    p99: float
    pmax: float
    saturation_fraction: float
    dynamic_range: float
    in_order_p50: float
    inter_order_p50: float
    in_vs_inter_ratio: float
    faint_band_snr: float
    verdict: str


def load_matrix():
    with MATRIX_JSON.open() as f:
        return json.load(f)


def metrics_for_frame(
    frame: np.ndarray,
    label: str,
    mcp: int,
    exp_ms: int,
    acc: int,
    eff_ms: int,
) -> FrameMetrics:
    h, w = frame.shape
    flat = frame.astype(np.float64).ravel()
    p50 = float(np.percentile(flat, 50))
    p99 = float(np.percentile(flat, 99))
    pmax = float(flat.max())

    # Saturation: relative to per-frame peak. Handles the 12-bit vs
    # 16-bit ambiguity without needing to probe the detector descriptor.
    sat_threshold = max(pmax * SAT_RELATIVE_THRESHOLD, 4000.0)
    saturation_fraction = float((flat >= sat_threshold).mean())

    # Split the frame into "bright" (in-order) vs "faint" (inter-order).
    # For a Mechelle 5000 echellogram, orders are thin stripes that cover
    # only ~3-5% of the detector area — so we need the top 3% as the
    # in-order proxy, not the top 30%. Using p97 handles the (rare) case
    # where the continuum is bright enough that the tail of the order
    # distribution blends into the background distribution too.
    bright_threshold = float(np.percentile(flat, 97))
    in_order_mask = flat >= bright_threshold
    inter_order_mask = ~in_order_mask

    in_order_p50 = float(np.percentile(flat[in_order_mask], 50))
    inter_order_p50 = float(np.percentile(flat[inter_order_mask], 50))
    in_vs_inter_ratio = in_order_p50 / max(inter_order_p50, 1.0)

    # Dynamic range: pmax / p50 (peak vs. background median). On a
    # working flat-field this is the range the MCP amplification +
    # accumulation has opened up — >10× means the orders are clearly
    # above the read-noise floor, and >100× means we're approaching the
    # detector's 16-bit ceiling.
    dynamic_range = pmax / max(p50, 1.0)

    # Faint-band SNR proxy: look at the rows near the UV end of the
    # detector (first ~300 rows — top of the detector per the
    # mechelle_5000.toml config where NIR is at top / UV at bottom).
    # Actually per config: "Higher Y → higher physical order number
    # (UV at bottom, NIR at top)" so UV is at y=H-300..H (bottom).
    uv_band = frame[h - 300 : h, :].astype(np.float64).ravel()
    # Pick the top 10% of UV-band pixels (the faintest orders that
    # reach here) and estimate SNR as median/std on those pixels.
    uv_bright_mask = uv_band >= np.percentile(uv_band, 90)
    uv_bright = uv_band[uv_bright_mask]
    faint_band_snr = float(np.median(uv_bright) / max(np.std(uv_band), 1.0))

    # Verdict heuristics, tuned for Mechelle 5000 echellograms where
    # orders occupy only ~3-5% of the detector:
    #
    #   heavily_saturated: >1% of pixels clipped (most orders at ceiling)
    #   noise_floor:       pmax < 2× p50 (no orders visible above noise)
    #   underexposed:      2 ≤ dynamic_range < 5 (orders dimly visible)
    #   usable:            5 ≤ dynamic_range < 50 AND sat < 1%
    #                      (orders clearly above noise, headroom preserved)
    #   saturated_orders:  saturation_fraction 0.05-1% OR dynamic_range ≥ 50
    #                      (bright orders saturated but peaks still signal)
    if saturation_fraction > 0.01:
        verdict = "heavily_saturated"
    elif pmax < 2 * p50:
        verdict = "noise_floor"
    elif dynamic_range < 5.0:
        verdict = "underexposed"
    elif saturation_fraction > 0.0005 or dynamic_range >= 50.0:
        verdict = "saturated_orders"
    else:
        verdict = "usable"

    return FrameMetrics(
        label=label,
        mcp_gain=mcp,
        exposure_ms=exp_ms,
        accumulate_count=acc,
        effective_ms=eff_ms,
        p50=p50,
        p99=p99,
        pmax=pmax,
        saturation_fraction=saturation_fraction,
        dynamic_range=dynamic_range,
        in_order_p50=in_order_p50,
        inter_order_p50=inter_order_p50,
        in_vs_inter_ratio=in_vs_inter_ratio,
        faint_band_snr=faint_band_snr,
        verdict=verdict,
    )


def main():
    matrix = load_matrix()
    report: list[dict] = []
    frames_for_plot: list[tuple[FrameMetrics, np.ndarray]] = []

    print(f"{'label':36s}  MCP   exp×acc=eff    p50    p99   pmax  sat%   dyn  ratio   snr  verdict")
    print("-" * 120)
    for entry in matrix:
        tiff_path = ROOT / entry["tiff"]
        frame = tifffile.imread(tiff_path)
        acc = int(entry.get("accumulate_count", 1))
        eff_ms = int(entry.get("effective_ms", entry["exposure_ms"] * acc))
        metrics = metrics_for_frame(
            frame,
            entry["label"],
            entry["mcp_gain"],
            entry["exposure_ms"],
            acc,
            eff_ms,
        )
        report.append({
            "label": metrics.label,
            "mcp_gain": metrics.mcp_gain,
            "exposure_ms": metrics.exposure_ms,
            "accumulate_count": metrics.accumulate_count,
            "effective_ms": metrics.effective_ms,
            "p50": metrics.p50,
            "p99": metrics.p99,
            "pmax": metrics.pmax,
            "saturation_fraction": metrics.saturation_fraction,
            "dynamic_range": metrics.dynamic_range,
            "in_order_p50": metrics.in_order_p50,
            "inter_order_p50": metrics.inter_order_p50,
            "in_vs_inter_ratio": metrics.in_vs_inter_ratio,
            "faint_band_snr": metrics.faint_band_snr,
            "verdict": metrics.verdict,
        })
        frames_for_plot.append((metrics, frame))

        print(
            f"{metrics.label:36s}  {metrics.mcp_gain:4d}  "
            f"{metrics.exposure_ms:5d}×{metrics.accumulate_count:2d}={metrics.effective_ms:6d}  "
            f"{metrics.p50:6.0f} {metrics.p99:6.0f} {metrics.pmax:6.0f}  "
            f"{100*metrics.saturation_fraction:4.1f}  {metrics.dynamic_range:4.1f}  "
            f"{metrics.in_vs_inter_ratio:5.1f}  {metrics.faint_band_snr:4.1f}  "
            f"{metrics.verdict}"
        )

    with REPORT_JSON.open("w") as f:
        json.dump(report, f, indent=2)
    print(f"\nWrote report to {REPORT_JSON.relative_to(ROOT.parent.parent)}")

    # Rectangular grid sized to frame count (2..3 cols, enough rows).
    n = len(frames_for_plot)
    cols = 3 if n > 4 else 2
    rows = (n + cols - 1) // cols
    fig, axes = plt.subplots(rows, cols, figsize=(6 * cols, 4.5 * rows),
                             squeeze=False)
    flat_axes = axes.flat
    for ax, (m, frame) in zip(flat_axes, frames_for_plot):
        log_frame = np.log10(frame.astype(np.float64) + 1.0)
        ax.imshow(log_frame, cmap="viridis", aspect="auto", origin="upper")
        title = f"{m.label}\np50={m.p50:.0f} p99={m.p99:.0f} sat={100*m.saturation_fraction:.1f}%  [{m.verdict}]"
        ax.set_title(title, fontsize=9)
        ax.set_xticks([])
        ax.set_yticks([])
    # Hide any unused axes in the final row.
    for ax in list(flat_axes)[n:]:
        ax.axis("off")
    fig.suptitle("DH3P DoE: log-scaled frame + verdict", fontsize=14)
    fig.tight_layout(rect=(0.0, 0.0, 1.0, 0.97))
    fig.savefig(PLOT_PATH, dpi=100)
    print(f"Wrote plot to {PLOT_PATH.relative_to(ROOT.parent.parent)}")

    # Summary verdict across matrix.
    by_verdict = {v: [m for m, _ in frames_for_plot if m.verdict == v]
                  for v in ("usable", "saturated_orders", "underexposed",
                            "heavily_saturated", "noise_floor")}
    print(f"\nVerdict summary: "
          f"{len(by_verdict['usable'])} usable / "
          f"{len(by_verdict['saturated_orders'])} saturated_orders / "
          f"{len(by_verdict['underexposed'])} underexposed / "
          f"{len(by_verdict['heavily_saturated'])} heavily_saturated / "
          f"{len(by_verdict['noise_floor'])} noise_floor")
    if by_verdict["usable"]:
        best = max(by_verdict["usable"], key=lambda m: m.in_vs_inter_ratio)
        print(f"Best usable frame: {best.label}  "
              f"(ratio={best.in_vs_inter_ratio:.1f}, dyn={best.dynamic_range:.1f})")
    elif by_verdict["saturated_orders"]:
        print("No fully-unsaturated frame — but saturated_orders frames may still "
              "be usable for trace detection (centroid survives clipping).")
    else:
        print("NO USABLE FRAMES — matrix needs iteration.")


if __name__ == "__main__":
    main()
