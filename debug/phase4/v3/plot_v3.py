#!/usr/bin/env python3
"""Plot the calibrated HgAr spectrum from the new-pipeline profile and
overlay the HG-2 atlas. Highlights which atlas lines are within 0.1 / 0.5
/ 1.0 nm of a predicted line in the profile's calibrated orders.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from PIL import Image

FRAME = Path("/home/brian/code/rust-daq/debug/phase4/v2/hgar-stack5x30s-median.tiff")
PROFILE = Path(sys.argv[1] if len(sys.argv) > 1 else
               "/home/brian/code/rust-daq/debug/phase4/v3/profile-prefilter.toml")
OUT = Path("/home/brian/code/rust-daq/debug/phase4/v3/hgar-v3.png")

HG_LINES = [
    (253.652, "Hg", 10000), (296.728, "Hg", 2000), (302.150, "Hg", 1500),
    (312.567, "Hg", 1800), (313.155, "Hg", 1200), (334.148, "Hg", 3000),
    (365.015, "Hg", 8000), (365.484, "Hg", 2000), (404.656, "Hg", 6000),
    (407.783, "Hg", 80), (435.833, "Hg", 9000), (546.074, "Hg", 10000),
    (576.960, "Hg", 4000), (579.066, "Hg", 3500),
]
AR_LINES = [
    (696.543, "Ar", 500), (706.722, "Ar", 300), (738.398, "Ar", 350),
    (750.387, "Ar", 600), (763.511, "Ar", 1000), (772.376, "Ar", 350),
    (794.818, "Ar", 400), (800.616, "Ar", 500), (810.369, "Ar", 350),
    (811.531, "Ar", 700), (826.452, "Ar", 400), (842.465, "Ar", 500),
]
ATLAS = HG_LINES + AR_LINES


def cheb_eval(coeffs: list[float], x: float) -> float:
    if not coeffs:
        return 0.0
    if len(coeffs) == 1:
        return coeffs[0]
    b1 = coeffs[-1]
    b2 = 0.0
    for c in reversed(coeffs[:-1]):
        b1, b2 = c + 2.0 * x * b1 - b2, b1
    return b1 - x * b2


def eval_monomial(coeffs, x):
    acc = 0.0
    xp = 1.0
    for c in coeffs:
        acc += c * xp
        xp *= x
    return acc


def order_wl(order: dict, pixel: float) -> float:
    wl = order["wavelength"]
    ds, de = wl["domain_start"], wl["domain_end"]
    if de <= ds:
        return float("nan")
    xn = max(-1.0, min(1.0, 2.0 * (pixel - ds) / (de - ds) - 1.0))
    basis = wl.get("basis", "chebyshev").lower()
    coeffs = wl["coefficients"]
    if basis == "chebyshev":
        return cheb_eval(coeffs, xn)
    return eval_monomial(coeffs, pixel)


def main() -> int:
    with open(PROFILE, "rb") as f:
        profile = tomllib.load(f)
    orders = [o for o in profile["orders"] if o.get("enabled", True)]
    frame = np.array(Image.open(FRAME), dtype=np.float64)
    height, _ = frame.shape

    # Extract 1D spectra per calibrated order via aperture sum
    fig, axes = plt.subplots(3, 1, figsize=(16, 11), sharex=False)
    ax_top, ax_mid, ax_bot = axes

    merged_wl = []
    merged_flux = []
    for o in orders:
        m = o.get("physical_order_number")
        if m is None:
            continue
        tr = o["trace"]
        t_coeffs = tr["coefficients"]
        t_ds, t_de = tr["domain_start"], tr["domain_end"]
        half = float(o.get("aperture_half_width_px", 4.0))
        wl_cfg = o["wavelength"]
        w_ds, w_de = wl_cfg["domain_start"], wl_cfg["domain_end"]
        px_start = int(max(w_ds, t_ds))
        px_end = int(min(w_de, t_de))
        xs = np.arange(px_start, px_end + 1)
        flux = np.zeros(len(xs))
        wls = np.zeros(len(xs))
        for i, x in enumerate(xs):
            yc = eval_monomial(t_coeffs, float(x))
            ylo = max(0, int(np.floor(yc - half)))
            yhi = min(height - 1, int(np.ceil(yc + half)))
            flux[i] = frame[ylo:yhi + 1, int(x)].sum()
            wls[i] = order_wl(o, float(x))

        # Continuum-subtract
        win = 101 if len(flux) > 201 else max(11, len(flux) // 10 * 2 + 1)
        if win % 2 == 0:
            win += 1
        cont = np.empty_like(flux)
        h = win // 2
        for i in range(len(flux)):
            cont[i] = np.median(flux[max(0, i - h):min(len(flux), i + h + 1)])
        flux_sub = np.clip(flux - cont, 0, None)
        merged_wl.append(wls)
        merged_flux.append(flux_sub)

    # Merge via per-wavelength-bin max
    all_wl = np.concatenate(merged_wl)
    all_flux = np.concatenate(merged_flux)
    wmin, wmax = 240.0, 910.0
    bin_w = 0.1
    nb = int((wmax - wmin) / bin_w) + 1
    binned = np.full(nb, np.nan)
    for w, f in zip(all_wl, all_flux):
        if wmin <= w <= wmax:
            i = int((w - wmin) / bin_w)
            if np.isnan(binned[i]) or f > binned[i]:
                binned[i] = f
    bin_wl = wmin + bin_w * (np.arange(nb) + 0.5)

    # Top panel: merged measured spectrum with atlas overlay
    ax_top.plot(bin_wl, binned, color="steelblue", linewidth=0.7)
    for (wl, sp, strength) in ATLAS:
        if wmin <= wl <= wmax:
            color = "crimson" if sp == "Hg" else "goldenrod"
            ax_top.axvline(wl, color=color, alpha=0.3, linewidth=0.6)
            if strength >= 400:
                ax_top.annotate(f"{wl:.1f}", (wl, 0.98),
                                xycoords=("data", "axes fraction"),
                                ha="center", va="top", fontsize=6,
                                color=color, rotation=90)
    ax_top.set_xlim(wmin, wmax)
    ymax = np.nanpercentile(binned, 99.9) * 1.2 if np.any(~np.isnan(binned)) else 100
    ax_top.set_ylim(0, ymax if ymax > 0 else 100)
    n_ok = sum(1 for o in orders if o.get("physical_order_number") is not None)
    ax_top.set_title(
        f"Mechelle 5000 + iStar HgAr (leabs-dev), {n_ok} calibrated orders — "
        f"bd-sw760 Phase A (global 2D Chebyshev + Cauchy Y(m))"
    )
    ax_top.set_ylabel("Flux (ADU − continuum)")
    ax_top.grid(True, alpha=0.3)

    # Middle panel: zoom 250–500 nm Hg UV cluster
    ax_mid.plot(bin_wl, binned, color="steelblue", linewidth=1.0)
    for (wl, sp, strength) in ATLAS:
        if sp == "Hg" and 250 <= wl <= 500:
            ax_mid.axvline(wl, color="crimson", alpha=0.5, linewidth=0.7)
            ax_mid.annotate(f"{wl:.3f}", (wl, 0.98),
                            xycoords=("data", "axes fraction"),
                            ha="center", va="top", fontsize=7,
                            color="crimson", rotation=90)
    ax_mid.set_xlim(250, 500)
    ymax_mid = np.nanpercentile(binned[(bin_wl >= 250) & (bin_wl <= 500)], 99.9) * 1.2
    ax_mid.set_ylim(0, ymax_mid if ymax_mid > 0 else 100)
    ax_mid.set_ylabel("Flux")
    ax_mid.set_title("Zoom: Hg UV cluster (250–500 nm, red ticks = NIST Hg atlas)")
    ax_mid.grid(True, alpha=0.3)

    # Bottom panel: zoom 500–900 nm Hg visible + Ar NIR
    ax_bot.plot(bin_wl, binned, color="steelblue", linewidth=1.0)
    for (wl, sp, strength) in ATLAS:
        if 500 <= wl <= 900:
            color = "crimson" if sp == "Hg" else "goldenrod"
            ax_bot.axvline(wl, color=color, alpha=0.5, linewidth=0.7)
            ax_bot.annotate(f"{wl:.3f}", (wl, 0.98),
                            xycoords=("data", "axes fraction"),
                            ha="center", va="top", fontsize=7,
                            color=color, rotation=90)
    ax_bot.set_xlim(500, 900)
    ymax_bot = np.nanpercentile(binned[(bin_wl >= 500) & (bin_wl <= 900)], 99.9) * 1.2
    ax_bot.set_ylim(0, ymax_bot if ymax_bot > 0 else 100)
    ax_bot.set_xlabel("Wavelength (nm)")
    ax_bot.set_ylabel("Flux")
    ax_bot.set_title("Zoom: Hg visible + Ar NIR (500–900 nm — red=Hg, gold=Ar)")
    ax_bot.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(OUT, dpi=140)
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
