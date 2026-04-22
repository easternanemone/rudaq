#!/usr/bin/env python3
# /// script
# dependencies = ["numpy", "tifffile", "matplotlib"]
# ///
"""
bd-cpph3 — Halogen-only flat-field DoE analysis.

Companion to analyze_d2.py. Halogen is a ~3000 K blackbody: peaks in
NIR (~900-1100 nm), falls off exponentially toward UV. For the Mechelle
5000 / iStar this means strong signal in the low-Y (NIR) orders m ≈
25-40 and nothing useful in the high-Y UV orders. Verdict logic
rewards NIR-end visibility; the D2 lamp covers the UV separately in
analyze_d2.py.

The "usable" criterion for a halogen flat is: NIR contrast ≥ 3× with
saturation below 1 % (headroom preserved for blaze extraction).

Outputs:
  * report.json
  * plot.png — log-scale frame previews + NIR-band row mean overlay.

Usage:  uv run analyze_halogen.py
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

# Mechelle 5000 + iStar, per config/calibration/mechelle_5000.toml:
# "Higher Y → higher physical order number (UV at bottom, NIR at top)"
# Detector is 2560×2160 (w × h). UV band = the bottom 360 rows.
UV_ROW_START = 1800
UV_ROW_END = 2160
NIR_ROW_START = 0
NIR_ROW_END = 360


@dataclass
class FrameMetrics:
    label: str
    mcp_gain: int
    exposure_ms: int
    accumulate_count: int
    effective_ms: int
    p50: float
    p97: float
    pmax: float
    saturation_fraction: float
    uv_band_p50: float
    uv_band_p97: float
    uv_band_contrast: float  # UV p97 / UV p50 — order visibility in UV
    nir_band_p50: float
    nir_band_p97: float
    nir_band_contrast: float  # NIR band reference
    uv_vs_nir_p97_ratio: float  # >1 means UV is brighter than NIR (D2 working)
    verdict: str


def metrics_for_frame(
    frame: np.ndarray,
    label: str,
    mcp: int,
    exp_ms: int,
    acc: int,
    eff_ms: int,
) -> FrameMetrics:
    flat = frame.astype(np.float64).ravel()
    p50 = float(np.percentile(flat, 50))
    p97 = float(np.percentile(flat, 97))
    pmax = float(flat.max())

    sat_threshold = max(pmax * 0.98, 4000.0)
    saturation_fraction = float((flat >= sat_threshold).mean())

    uv_band = frame[UV_ROW_START:UV_ROW_END, :].astype(np.float64).ravel()
    nir_band = frame[NIR_ROW_START:NIR_ROW_END, :].astype(np.float64).ravel()

    uv_p50 = float(np.percentile(uv_band, 50))
    uv_p97 = float(np.percentile(uv_band, 97))
    uv_contrast = uv_p97 / max(uv_p50, 1.0)

    nir_p50 = float(np.percentile(nir_band, 50))
    nir_p97 = float(np.percentile(nir_band, 97))
    nir_contrast = nir_p97 / max(nir_p50, 1.0)

    uv_vs_nir = uv_p97 / max(nir_p97, 1.0)

    # Verdicts for halogen-only are NIR-centric:
    #   nir_ready:         NIR contrast ≥ 3× AND sat < 1 % — orders visible in NIR band.
    #   nir_saturated:     sat > 1 % — too much gain/exposure for halogen.
    #   nir_underexposed:  NIR contrast < 2 — need more integration or gain.
    #   uv_dominant:       uv_vs_nir > 1 — unexpected for halogen (D2 still on?).
    #   marginal:          NIR contrast in [2, 3) — may work but risk of low SNR.
    if saturation_fraction > 0.01:
        verdict = "nir_saturated"
    elif nir_contrast >= 3.0 and uv_vs_nir <= 1.0:
        verdict = "nir_ready"
    elif nir_contrast < 2.0:
        verdict = "nir_underexposed"
    elif uv_vs_nir > 1.0:
        verdict = "uv_dominant"
    else:
        verdict = "marginal"

    return FrameMetrics(
        label=label,
        mcp_gain=mcp,
        exposure_ms=exp_ms,
        accumulate_count=acc,
        effective_ms=eff_ms,
        p50=p50,
        p97=p97,
        pmax=pmax,
        saturation_fraction=saturation_fraction,
        uv_band_p50=uv_p50,
        uv_band_p97=uv_p97,
        uv_band_contrast=uv_contrast,
        nir_band_p50=nir_p50,
        nir_band_p97=nir_p97,
        nir_band_contrast=nir_contrast,
        uv_vs_nir_p97_ratio=uv_vs_nir,
        verdict=verdict,
    )


def main():
    with MATRIX_JSON.open() as f:
        matrix = json.load(f)

    report: list[dict] = []
    frames_for_plot: list[tuple[FrameMetrics, np.ndarray]] = []

    print(
        f"{'label':30s} MCP  eff_ms   p50    p97   pmax  sat%  "
        f"uv_c  nir_c  uv/nir  verdict"
    )
    print("-" * 110)
    for entry in matrix:
        tiff_path = ROOT / entry["tiff"]
        frame = tifffile.imread(tiff_path)
        m = metrics_for_frame(
            frame,
            entry["label"],
            entry["mcp_gain"],
            entry["exposure_ms"],
            entry["accumulate_count"],
            entry["effective_ms"],
        )
        report.append({k: getattr(m, k) for k in m.__annotations__})
        frames_for_plot.append((m, frame))
        print(
            f"{m.label:30s} {m.mcp_gain:4d} {m.effective_ms:7d}  "
            f"{m.p50:5.0f} {m.p97:6.0f} {m.pmax:5.0f}  "
            f"{100 * m.saturation_fraction:4.1f}  "
            f"{m.uv_band_contrast:4.1f}  {m.nir_band_contrast:4.1f}   "
            f"{m.uv_vs_nir_p97_ratio:5.2f}   {m.verdict}"
        )

    with REPORT_JSON.open("w") as f:
        json.dump(report, f, indent=2)

    n = len(frames_for_plot)
    cols = 3 if n > 4 else 2
    rows = (n + cols - 1) // cols
    # Two stacked grids: frame previews + UV row-mean overlays.
    fig, axes = plt.subplots(rows + 1, cols, figsize=(6 * cols, 4.5 * (rows + 1)), squeeze=False)
    flat_axes = axes.flat
    for ax, (m, frame) in zip(flat_axes, frames_for_plot):
        ax.imshow(np.log10(frame.astype(np.float64) + 1.0), cmap="viridis", aspect="auto")
        ax.set_title(
            f"{m.label}\n"
            f"uv_c={m.uv_band_contrast:.1f} uv/nir={m.uv_vs_nir_p97_ratio:.2f} "
            f"sat={100 * m.saturation_fraction:.1f}%  [{m.verdict}]",
            fontsize=9,
        )
        ax.set_xticks([])
        ax.set_yticks([])
        ax.axhline(UV_ROW_START, color="red", linewidth=0.5, alpha=0.6)
        ax.axhline(UV_ROW_END - 1, color="red", linewidth=0.5, alpha=0.6)

    # Hide unused cells in frame-preview rows.
    for ax in list(flat_axes)[n : rows * cols]:
        ax.axis("off")

    # Bottom row: UV-band column-mean cross-sections (row-averaged inside the UV band).
    if rows * cols < (rows + 1) * cols:
        uv_ax = axes[rows][cols // 2] if cols >= 2 else axes[rows][0]
        for m, frame in frames_for_plot:
            uv_slab = frame[UV_ROW_START:UV_ROW_END, :].astype(np.float64)
            uv_profile = uv_slab.mean(axis=0)
            uv_ax.plot(uv_profile, label=m.label, alpha=0.7)
        uv_ax.set_title("UV-band mean cross-section (y=1800-2160)", fontsize=10)
        uv_ax.set_xlabel("x (dispersion pixel)")
        uv_ax.set_ylabel("mean counts")
        uv_ax.legend(fontsize=7, loc="upper right")
        # Hide other cells in the bottom row.
        for col_i in range(cols):
            if not (cols >= 2 and col_i == cols // 2) and not (cols < 2 and col_i == 0):
                axes[rows][col_i].axis("off")

    fig.suptitle("Halogen-only DoE: frames + UV-band cross-sections", fontsize=14)
    fig.tight_layout(rect=(0.0, 0.0, 1.0, 0.97))
    fig.savefig(PLOT_PATH, dpi=100)
    print(f"\nWrote report to {REPORT_JSON.relative_to(ROOT.parent.parent)}")
    print(f"Wrote plot to {PLOT_PATH.relative_to(ROOT.parent.parent)}")

    by_verdict: dict[str, list[FrameMetrics]] = {}
    for m, _ in frames_for_plot:
        by_verdict.setdefault(m.verdict, []).append(m)

    print("\nVerdict breakdown: " + ", ".join(
        f"{len(v)} {k}" for k, v in by_verdict.items()
    ))
    if by_verdict.get("nir_ready"):
        best = max(by_verdict["nir_ready"], key=lambda m: m.nir_band_contrast)
        print(
            f"Best NIR-ready frame: {best.label} "
            f"(nir_contrast={best.nir_band_contrast:.2f}, "
            f"uv/nir={best.uv_vs_nir_p97_ratio:.2f})"
        )
    elif by_verdict.get("marginal"):
        best = max(by_verdict["marginal"], key=lambda m: m.nir_band_contrast)
        print(f"No nir_ready frames — best marginal: {best.label}")
    else:
        print("NO NIR-ready OR marginal frames — matrix needs iteration.")


if __name__ == "__main__":
    main()
