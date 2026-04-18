#!/usr/bin/env python3
"""Find the spatial (x, y) location of the brightest peaks in the raw
HgAr stack and report which calibrated order (if any) they fall in.

This separates 'Hg line not in frame' from 'Hg line in frame but no
calibrated order covers it'.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

import numpy as np
from PIL import Image

FRAME = Path("/home/brian/code/rust-daq/debug/phase4/v2/hgar-stack5x30s-median.tiff")
PROFILE = Path(sys.argv[1] if len(sys.argv) > 1 else
               "/home/brian/code/rust-daq/debug/phase4/v3/profile-gc20139.toml")


def eval_monomial(coeffs, x):
    acc, xp = 0.0, 1.0
    for c in coeffs:
        acc += c * xp
        xp *= x
    return acc


def cheb_eval(coeffs, x):
    if not coeffs:
        return 0.0
    if len(coeffs) == 1:
        return coeffs[0]
    b1 = coeffs[-1]
    b2 = 0.0
    for c in reversed(coeffs[:-1]):
        b1, b2 = c + 2.0 * x * b1 - b2, b1
    return b1 - x * b2


def main() -> int:
    img = np.array(Image.open(FRAME), dtype=np.float64)
    h, w = img.shape
    print(f"frame {w}x{h}, max={img.max():.0f}")

    # Find top N brightest pixels that aren't on the edge
    mask = np.ones_like(img, dtype=bool)
    mask[:20, :] = False
    mask[-20:, :] = False
    mask[:, :20] = False
    mask[:, -20:] = False
    flat = img.copy()
    flat[~mask] = 0

    # Find peaks above 50% of the max flux, collapsed into distinct centroids
    threshold = img.max() * 0.2
    peaks = []
    seen = set()
    while len(peaks) < 40:
        idx = np.unravel_index(np.argmax(flat), flat.shape)
        y, x = idx
        val = flat[y, x]
        if val < threshold:
            break
        # Check if this is in a already-reported region (within 20 px)
        near = any(abs(y - py) < 20 and abs(x - px) < 40 for py, px, _ in peaks)
        if not near:
            peaks.append((y, x, val))
        # Suppress this region so the next argmax finds a different peak
        flat[max(0, y - 20):min(h, y + 21), max(0, x - 40):min(w, x + 41)] = 0

    print(f"\nTop {len(peaks)} bright peaks (threshold = {threshold:.0f}):")
    with open(PROFILE, "rb") as f:
        profile = tomllib.load(f)
    orders = profile["orders"]

    HG = [253.652, 296.728, 302.150, 312.567, 313.155, 334.148, 365.015, 365.484,
          404.656, 407.783, 435.833, 491.607, 546.074, 576.960, 579.066]
    AR = [696.543, 706.722, 738.398, 750.387, 763.511, 772.376, 800.616, 811.531]

    print(f"{'idx':>3s} {'y':>5s} {'x':>5s} {'ADU':>8s} {'nearest trace':>20s} {'λ(order)':>10s} {'nearest atlas':>20s}")
    for i, (y, x, val) in enumerate(peaks):
        # Find traces whose polynomial eval at x gives y within ±8 px
        matching = []
        for o in orders:
            if not o.get("enabled", True):
                continue
            tr = o["trace"]
            coeffs = tr["coefficients"]
            ds, de = tr["domain_start"], tr["domain_end"]
            if x < ds or x > de:
                continue
            y_trace = eval_monomial(coeffs, float(x))
            if abs(y - y_trace) < 8:
                wl_cfg = o["wavelength"]
                w_ds, w_de = wl_cfg["domain_start"], wl_cfg["domain_end"]
                if w_ds <= x <= w_de:
                    xn = 2.0 * (x - w_ds) / (w_de - w_ds) - 1.0
                    xn = max(-1.0, min(1.0, xn))
                    basis = wl_cfg.get("basis", "chebyshev").lower()
                    coeffs_wl = wl_cfg["coefficients"]
                    if basis == "chebyshev":
                        lam = cheb_eval(coeffs_wl, xn)
                    else:
                        lam = eval_monomial(coeffs_wl, float(x))
                else:
                    lam = float("nan")
                matching.append((o.get("relative_index"), o.get("physical_order_number"),
                                 y_trace, lam))
        if matching:
            ri, m, yt, lam = matching[0]
            near_atlas = min((abs(lam - a), a, 'Hg') for a in HG) if lam == lam else (0, 0, '')
            near_atlas_ar = min((abs(lam - a), a, 'Ar') for a in AR) if lam == lam else (0, 0, '')
            best = min(near_atlas, near_atlas_ar)
            print(f"{i:3d} {y:5d} {x:5d} {val:8.0f} rel={ri:2d} m={m:3d} y_t={yt:5.0f} "
                  f"λ={lam:8.2f}  {best[2]} {best[1]:7.3f} Δ{best[0]:5.2f}")
        else:
            # Uncalibrated — but maybe a trace at this y existed and wasn't calibrated
            # search all traces by trace polynomial centroid within ±8
            # (can't identify which 'physical' order)
            print(f"{i:3d} {y:5d} {x:5d} {val:8.0f} <no calibrated order covers this pixel>")

    return 0


if __name__ == "__main__":
    sys.exit(main())
