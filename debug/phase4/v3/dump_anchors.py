#!/usr/bin/env python3
"""Dump (y_centroid, m) anchors from a profile and evaluate Cauchy fit quality.

Helps diagnose whether trace y-centroids actually follow the Cauchy series
y(m) = a + b/m^2 + c/m^4 or whether a higher-order model is needed.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

import numpy as np


def eval_trace(coeffs: list[float], domain: tuple[float, float], x: float) -> float:
    """Evaluate a monomial trace polynomial at pixel x."""
    lo, hi = domain
    if x < lo or x > hi:
        return float("nan")
    acc = 0.0
    xp = 1.0
    for c in coeffs:
        acc += c * xp
        xp *= x
    return acc


def fit_cauchy(anchors: list[tuple[float, int]]) -> dict:
    """Fit y = a + b*u + c*u^2 where u = 1/m^2."""
    us = np.array([1.0 / (m * m) for _, m in anchors], dtype=float)
    ys = np.array([y for y, _ in anchors], dtype=float)
    A = np.column_stack([np.ones_like(us), us, us * us])
    coeffs, *_ = np.linalg.lstsq(A, ys, rcond=None)
    a, b, c = coeffs
    preds = A @ coeffs
    resids = ys - preds
    return {
        "a": a,
        "b": b,
        "c": c,
        "rms": float(np.sqrt(np.mean(resids ** 2))),
        "resids": dict(zip([m for _, m in anchors], resids)),
    }


def fit_cauchy_extended(anchors: list[tuple[float, int]]) -> dict:
    """Fit y = a + b*u + c*u^2 + d*u^3 where u = 1/m^2 (adds d/m^6 term)."""
    us = np.array([1.0 / (m * m) for _, m in anchors], dtype=float)
    ys = np.array([y for y, _ in anchors], dtype=float)
    A = np.column_stack([np.ones_like(us), us, us * us, us * us * us])
    coeffs, *_ = np.linalg.lstsq(A, ys, rcond=None)
    preds = A @ coeffs
    resids = ys - preds
    return {
        "coeffs": coeffs.tolist(),
        "rms": float(np.sqrt(np.mean(resids ** 2))),
    }


def main() -> int:
    path = Path(sys.argv[1] if len(sys.argv) > 1 else "debug/phase4/v3/profile-prefilter.toml")
    with open(path, "rb") as f:
        data = tomllib.load(f)

    det = data.get("detector", {})
    width = det.get("width", 2560)
    x_mid = width / 2.0

    anchors: list[tuple[float, int]] = []
    for o in data["orders"]:
        m = o.get("physical_order_number")
        if m is None:
            continue
        tr = o.get("trace", {})
        coeffs = tr.get("coefficients", [])
        ds = tr.get("domain_start", 0.0)
        de = tr.get("domain_end", width)
        y = eval_trace(coeffs, (ds, de), x_mid)
        if not np.isnan(y):
            anchors.append((y, m))

    anchors.sort(key=lambda t: t[1])  # sort by m
    print(f"{len(anchors)} anchors (sorted by m):")
    print(f"{'m':>5s} {'y_centroid':>10s}")
    for y, m in anchors:
        print(f"{m:5d} {y:10.2f}")

    # Cauchy fit (3-parameter)
    fit = fit_cauchy(anchors)
    print()
    print(f"Cauchy 3-parameter fit: a={fit['a']:.4f}  b={fit['b']:.4f}  c={fit['c']:.4f}")
    print(f"  RMS = {fit['rms']:.3f} px")
    print(f"  Per-m residuals:")
    for m, r in sorted(fit["resids"].items()):
        flag = " ←outlier" if abs(r) > 2.0 * fit["rms"] else ""
        print(f"    m={m}: {r:+.3f}{flag}")

    # Cauchy fit (4-parameter = adds 1/m^6)
    fit4 = fit_cauchy_extended(anchors)
    print()
    print(f"Extended (1/m^6 term): RMS = {fit4['rms']:.3f} px")
    print(f"  coeffs: {[f'{c:.4e}' for c in fit4['coeffs']]}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
