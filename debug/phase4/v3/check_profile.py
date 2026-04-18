#!/usr/bin/env python3
"""Quantitative check of an echelle calibration profile against the HgAr atlas.

Loads a TOML profile and reports, per calibrated order:
  - physical_order_number
  - pixel domain
  - evaluated wavelength range
  - midpoint wavelength and the closest Hg/Ar atlas line within 1 nm
"""

from __future__ import annotations

import math
import sys
import tomllib
from pathlib import Path

HG2_ATLAS = [
    (253.652, "Hg"), (296.728, "Hg"), (302.150, "Hg"),
    (312.567, "Hg"), (313.155, "Hg"), (334.148, "Hg"),
    (365.015, "Hg"), (365.484, "Hg"), (404.656, "Hg"),
    (407.783, "Hg"), (435.833, "Hg"), (491.607, "Hg"),
    (546.074, "Hg"), (576.960, "Hg"), (579.066, "Hg"),
    (696.543, "Ar"), (706.722, "Ar"), (738.398, "Ar"),
    (750.387, "Ar"), (763.511, "Ar"), (772.376, "Ar"),
    (794.818, "Ar"), (800.616, "Ar"), (810.369, "Ar"),
    (811.531, "Ar"), (826.452, "Ar"), (842.465, "Ar"),
    (852.144, "Ar"), (866.794, "Ar"), (912.297, "Ar"),
]


def cheb_eval(coeffs: list[float], x: float) -> float:
    """Evaluate Chebyshev series on pre-normalized x ∈ [-1, 1]."""
    if not coeffs:
        return 0.0
    if len(coeffs) == 1:
        return coeffs[0]
    b1 = coeffs[-1]
    b2 = 0.0
    for c in reversed(coeffs[:-1]):
        b1, b2 = c + 2.0 * x * b1 - b2, b1
    return b1 - x * b2


def eval_wl(order: dict, pixel: float) -> float:
    wl = order["wavelength"]
    ds, de = wl["domain_start"], wl["domain_end"]
    xn = max(-1.0, min(1.0, 2.0 * (pixel - ds) / (de - ds) - 1.0))
    basis = wl.get("basis", "chebyshev").lower()
    coeffs = wl["coefficients"]
    if basis == "chebyshev":
        return cheb_eval(coeffs, xn)
    acc = 0.0
    xp = 1.0
    for c in coeffs:
        acc += c * xp
        xp *= pixel
    return acc


def closest_atlas(lam: float, max_delta: float = 2.0):
    best = None
    for wl, sp in HG2_ATLAS:
        d = abs(lam - wl)
        if d <= max_delta and (best is None or d < best[2]):
            best = (wl, sp, d)
    return best


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check_profile.py <profile.toml>", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    with open(path, "rb") as f:
        data = tomllib.load(f)

    orders = [o for o in data.get("orders", []) if o.get("enabled", True)]
    print(f"{path}: {len(orders)} enabled orders")
    print(f"{'rel':>4s} {'m':>4s} {'px_min':>7s} {'px_mid':>7s} {'px_max':>7s} "
          f"{'λ_mid':>9s} {'nearest atlas':>22s}")
    print("-" * 70)

    per_m_hits = {}
    for o in orders:
        m = o.get("physical_order_number", None)
        wl = o["wavelength"]
        ds, de = wl["domain_start"], wl["domain_end"]
        mid = 0.5 * (ds + de)
        lam_mid = eval_wl(o, mid)
        lam_lo = eval_wl(o, ds)
        lam_hi = eval_wl(o, de)
        near = closest_atlas(lam_mid, 5.0)
        near_str = (
            f"{near[1]} {near[0]:.3f} (Δ{near[2]:.3f})" if near else "---"
        )
        ri = o.get("relative_index", -1)
        print(f"{ri:4d} {m!s:>4s} {ds:7.0f} {mid:7.0f} {de:7.0f} "
              f"{lam_mid:9.2f} {near_str:>22s}")
        if near and near[2] < 1.0:
            per_m_hits.setdefault(near[1], []).append(near[0])

    print()
    print("Matches within 1.0 nm (atlas_wavelength : count):")
    for sp in ("Hg", "Ar"):
        hits = per_m_hits.get(sp, [])
        if not hits:
            print(f"  {sp}: none")
            continue
        hits_sorted = sorted(set(hits))
        print(f"  {sp}: {len(hits_sorted)} unique lines → {hits_sorted}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
