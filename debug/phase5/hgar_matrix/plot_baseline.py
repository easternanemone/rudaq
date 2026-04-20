#!/usr/bin/env python3
"""Plot per-frame echelle arc extractions from the bd-g22gu.2.2.2 baseline run.

Consumes `baseline_matches.json` + `baseline_details.json` written by the
`echelle_hgar_doe_baseline` integration test and emits one PNG per frame
(`plots/<label>.png`) plus an overall summary (`plots/_summary.png`).

Usage:
    cd debug/phase5/hgar_matrix
    python3 plot_baseline.py                 # all 9 frames
    python3 plot_baseline.py g500_t3000ms    # substring match, single frame

Rationale: the scalar numbers in `baseline_matches.json` are consistent
with memo-dtw-line-id.md §3's sparse-regime hypothesis (many single-line
fallback fits with trivially-zero RMS), but only a plot shows *where on
the dispersion axis* the matched atlas lines fall, whether the echelle-
equation seed drifts at order extremes, and whether DTW would have any
remaining pixels to win on. This script produces those visuals.
"""
from __future__ import annotations

import json
import pathlib
import sys
from dataclasses import dataclass

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

HERE = pathlib.Path(__file__).parent
MATCHES = HERE / "baseline_matches.json"
DETAILS = HERE / "baseline_details.json"
PLOTS = HERE / "plots"

# Panel grid shape for per-frame PNG — top N calibrated orders by
# `lines_matched`. Six is enough to see the failure-mode diversity
# without crowding at 14×10 in figsize.
PANEL_ROWS = 3
PANEL_COLS = 2
TOP_ORDERS = PANEL_ROWS * PANEL_COLS


def load_data():
    with MATCHES.open() as fh:
        matches = json.load(fh)
    with DETAILS.open() as fh:
        details = json.load(fh)
    match_by_label = {m["label"]: m for m in matches}
    return matches, details, match_by_label


def order_sort_key(order):
    """Rank successful orders first (by -lines_matched, then rms), then
    failed orders by -lines_detected so the richest-peak failures lead."""
    if order["success"]:
        rms = order["rms_nm"] if np.isfinite(order["rms_nm"]) else 1e9
        return (0, -order["lines_matched"], rms)
    return (1, -order["lines_detected"], 0.0)


def plot_order_panel(ax, order, atlas_tol_nm: float) -> None:
    flux = np.asarray(order["flux"], dtype=np.float64)
    px = np.arange(order["disp_start"], order["disp_end"] + 1, dtype=np.float64)
    wl = np.asarray(order["wavelengths_nm"], dtype=np.float64)
    if flux.size != px.size:
        # Defensive: test guarantees equality but sampled-lookup orders
        # could in principle diverge; let the panel degrade gracefully.
        px = np.arange(flux.size, dtype=np.float64) + order["disp_start"]

    ax.plot(px, flux, color="black", lw=0.6)

    # Detected peaks — blue ticks at bottom of panel, matched peaks in green.
    ymax = float(flux.max()) if flux.size else 1.0
    if ymax <= 0:
        ymax = 1.0
    for peak in order["detected_peaks"]:
        px_d = peak["pixel_center"]
        matched = peak["nearest_atlas_nm"] is not None
        color = "green" if matched else "steelblue"
        alpha = 0.9 if matched else 0.25
        lw = 1.0 if matched else 0.4
        ax.axvline(px_d, ymin=0.0, ymax=0.08, color=color, alpha=alpha, lw=lw)
        if peak.get("saturated"):
            ax.plot(px_d, ymax * 0.95, marker="v", color="orange", ms=4)

    # Atlas projections — red dashed verticals at the *predicted* pixel
    # of each atlas line whose λ lands inside this order's range.
    for proj in order["atlas_projections"]:
        if proj["pixel"] is None:
            continue
        ax.axvline(proj["pixel"], color="crimson", alpha=0.55, lw=0.8, linestyle="--")
        ax.annotate(
            f"{proj['wavelength_nm']:.2f}",
            xy=(proj["pixel"], ymax * 0.92),
            xycoords="data",
            rotation=90,
            fontsize=6,
            color="crimson",
            ha="right",
            va="top",
        )

    wl_lo = wl[np.isfinite(wl)].min() if np.isfinite(wl).any() else float("nan")
    wl_hi = wl[np.isfinite(wl)].max() if np.isfinite(wl).any() else float("nan")
    status = "OK" if order["success"] else "FAIL"
    src = order["wavelength_source"]  # "fit" or "seed"
    m = order.get("physical_order_m", "?")
    title = (
        f"[{status}] order {order['order_index']}  m≈{m}  "
        f"λ_{{{src}}}={wl_lo:.1f}–{wl_hi:.1f} nm  "
        f"detect={order['lines_detected']}  "
        f"match={order['lines_matched']}  "
        f"used={order['lines_used']}  "
        f"rms={order['rms_nm']:.3g} nm"
    )
    color = "black" if order["success"] else "firebrick"
    ax.set_title(title, fontsize=9, color=color)
    ax.set_xlabel("pixel")
    ax.set_ylabel("flux (ADU)")
    ax.grid(True, alpha=0.15)


def plot_frame(frame_details, match_summary, atlas_tol_nm: float, out_path: pathlib.Path) -> int:
    orders = frame_details["orders"]
    if not orders:
        return 0
    successes = sorted(
        (o for o in orders if o["success"]), key=order_sort_key
    )
    failures = sorted(
        (o for o in orders if not o["success"]), key=order_sort_key
    )
    # Left column: best-calibrated orders. Right column: richest-peak
    # failed orders. Three rows tall so we compare 3 successes vs 3
    # failures side-by-side.
    left = successes[:PANEL_ROWS]
    right = failures[:PANEL_ROWS]

    fig, axes = plt.subplots(
        PANEL_ROWS, 2, figsize=(16, 11), constrained_layout=True
    )
    # axes is (rows, 2); column 0 = success, column 1 = failure.
    for row, (s, f) in enumerate(
        zip(
            left + [None] * (PANEL_ROWS - len(left)),
            right + [None] * (PANEL_ROWS - len(right)),
        )
    ):
        for col, order in enumerate((s, f)):
            ax = axes[row, col]
            if order is None:
                ax.axis("off")
                continue
            plot_order_panel(ax, order, atlas_tol_nm)

    title = (
        f"{frame_details['label']}  "
        f"gain={frame_details['mcp_gain']}  "
        f"exp={frame_details['exposure_ms']} ms  |  "
        f"orders cal'd {match_summary['n_orders_calibrated']}/{match_summary['n_orders_detected']}  "
        f"total lines {match_summary['total_lines_detected']}→{match_summary['total_lines_matched']}  "
        f"overall_rms={match_summary['overall_rms_nm']:.3f} nm  "
        f"(atlas match ±{atlas_tol_nm:.1f} nm)\n"
        f"Left column: top-3 calibrated orders (λ from fit).  "
        f"Right column: top-3 failed orders by peak count (λ from echelle-equation seed)."
    )
    fig.suptitle(title, fontsize=10)

    fig.savefig(out_path, dpi=110)
    plt.close(fig)
    return len(left) + len(right)


def plot_summary(matches: list, out_path: pathlib.Path) -> None:
    gains = np.array([m["mcp_gain"] for m in matches])
    exps = np.array([m["exposure_ms"] for m in matches])
    rms = np.array([m["overall_rms_nm"] for m in matches])
    matched = np.array([m["total_lines_matched"] for m in matches])
    orders_cal = np.array([m["n_orders_calibrated"] for m in matches])
    orders_tot = np.array([m["n_orders_detected"] for m in matches])

    fig, (ax0, ax1) = plt.subplots(1, 2, figsize=(14, 6), constrained_layout=True)

    # Left: overall RMS vs exposure × gain (scatter; size = total matches).
    sc = ax0.scatter(
        gains, exps, c=rms, s=25 + 3 * matched, cmap="viridis_r", edgecolor="black"
    )
    for m in matches:
        ax0.annotate(
            f"{m['label'].removeprefix('hgar_')}\nrms={m['overall_rms_nm']:.2f}\nmatch={m['total_lines_matched']}",
            xy=(m["mcp_gain"], m["exposure_ms"]),
            xytext=(5, 5),
            textcoords="offset points",
            fontsize=7,
        )
    ax0.set_xlabel("MCP gain")
    ax0.set_ylabel("exposure (ms)")
    ax0.set_yscale("log")
    ax0.set_title("Overall RMS vs (gain, exposure) — size ∝ matched lines")
    plt.colorbar(sc, ax=ax0, label="overall_rms_nm")
    ax0.grid(True, alpha=0.2)

    # Right: orders calibrated per frame, grouped bars.
    idx = np.arange(len(matches))
    ax1.bar(idx - 0.2, orders_tot, width=0.4, color="lightgray", label="detected")
    ax1.bar(idx + 0.2, orders_cal, width=0.4, color="steelblue", label="calibrated")
    ax1.set_xticks(idx)
    ax1.set_xticklabels([m["label"].removeprefix("hgar_") for m in matches], rotation=45, ha="right", fontsize=8)
    ax1.set_ylabel("order count")
    ax1.set_title("Orders detected vs calibrated per frame")
    ax1.legend()
    ax1.grid(True, axis="y", alpha=0.2)

    fig.suptitle("HgAr DoE matrix baseline — bd-g22gu.2.2.2 step 1", fontsize=12)
    fig.savefig(out_path, dpi=110)
    plt.close(fig)


def main(argv: list[str]) -> int:
    matches, details, match_by_label = load_data()
    PLOTS.mkdir(exist_ok=True)

    wanted = argv[1:]
    targets = details if not wanted else [
        d for d in details if any(w in d["label"] for w in wanted)
    ]
    if not targets:
        print(f"no frames matched {wanted!r}", file=sys.stderr)
        return 1

    n_drawn = 0
    for frame in targets:
        summary = match_by_label[frame["label"]]
        out = PLOTS / f"{frame['label']}.png"
        drawn = plot_frame(frame, summary, frame["atlas_match_tolerance_nm"], out)
        print(f"  {frame['label']}: {drawn} panels → {out.name}")
        n_drawn += drawn

    summary_out = PLOTS / "_summary.png"
    plot_summary(matches, summary_out)
    print(f"  summary → {summary_out.name}")
    print(f"done ({n_drawn} order panels across {len(targets)} frames)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
