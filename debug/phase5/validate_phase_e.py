#!/usr/bin/env python3
"""Phase E (bd-w8wa6) end-to-end validation on the real DH3P flat.

Port of the key blaze.rs algorithms in Python so we can visualize the
DH3P continuum fit and per-order blaze curves without needing to build
a bespoke Rust binary.

Reads:
  - debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff  (real DH3P flat)
  - debug/phase5/profile-march18.toml                  (calibration profile)

Produces:
  - debug/phase5/phaseE-dh3p-continuum.png  (overlay of per-order flux,
    fitted C_lamp, resulting B_i(x) blaze curves)
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from PIL import Image

FLAT = Path("debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff")
# Use the NO-FLAT arc-detected profile: 12 orders span UV→visible
# (m=46–99), not just the 5 visible orders the DH3P flat trace detector
# finds. The DH3P continuum fit needs coverage across the full bandpass;
# weakly-illuminated UV orders still carry samples of C_lamp(λ) there.
PROFILE = Path("debug/phase5/profile-march18-noflat.toml")
OUT = Path("debug/phase5/phaseE-dh3p-continuum.png")


def eval_monomial(coeffs, x):
    acc, xp = 0.0, 1.0
    for c in coeffs:
        acc += c * xp
        xp *= x
    return acc


def cheb_eval(coeffs, xn):
    if not coeffs:
        return 0.0
    if len(coeffs) == 1:
        return coeffs[0]
    b1, b2 = coeffs[-1], 0.0
    for c in reversed(coeffs[:-1]):
        b1, b2 = c + 2.0 * xn * b1 - b2, b1
    return b1 - xn * b2


def eval_wl(order, pixel):
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


def extract_flat_flux(frame, order, aperture=4):
    """Python port of Rust's simple-sum extraction along a trace."""
    tr = order["trace"]
    tc = tr["coefficients"]
    ds, de = int(tr["domain_start"]), int(tr["domain_end"])
    h, _ = frame.shape
    xs = np.arange(max(0, ds), min(frame.shape[1] - 1, de) + 1)
    flux = np.zeros(len(xs))
    wls = np.zeros(len(xs))
    for i, x in enumerate(xs):
        yc = eval_monomial(tc, float(x))
        ylo = max(0, int(np.floor(yc - aperture)))
        yhi = min(h - 1, int(np.ceil(yc + aperture)))
        flux[i] = frame[ylo : yhi + 1, int(x)].sum()
        wls[i] = eval_wl(order, float(x))
    return xs, flux, wls


def fit_dh3p_continuum(orders_data, n_knots=64, sigma=3.0, max_iters=5):
    """Python port of blaze.rs::fit_dh3p_continuum."""
    samples = []
    for _xs, flux, wl in orders_data:
        mask = np.isfinite(flux) & np.isfinite(wl) & (flux > 0) & (wl > 100) & (wl < 2000)
        samples.extend(zip(wl[mask], flux[mask]))
    samples = np.array(sorted(samples))
    if len(samples) < n_knots * 2:
        return None
    wl_s = samples[:, 0]
    fx_s = samples[:, 1]
    lo, hi = wl_s[0], wl_s[-1]
    knots = np.linspace(lo, hi, n_knots)
    kept = np.ones(len(samples), dtype=bool)
    window = 1.5 * (knots[1] - knots[0])

    for _ in range(max_iters):
        # Median-in-window per knot
        knot_fluxes = np.zeros(n_knots)
        for j, k in enumerate(knots):
            sel = kept & (np.abs(wl_s - k) <= window)
            if sel.any():
                knot_fluxes[j] = np.median(fx_s[sel])
        # Piecewise-linear interp at kept sample wavelengths
        interp = np.interp(wl_s, knots, knot_fluxes)
        residuals = fx_s - interp
        mean_r = np.mean(residuals[kept])
        std_r = np.std(residuals[kept])
        if std_r <= 0:
            break
        thresh = sigma * std_r
        new_kept = kept & (residuals - mean_r <= thresh)
        if (new_kept == kept).all():
            break
        kept = new_kept
    # Final fit
    for j, k in enumerate(knots):
        sel = kept & (np.abs(wl_s - k) <= window)
        if sel.any():
            knot_fluxes[j] = np.median(fx_s[sel])
    return {
        "knots": knots,
        "fluxes": knot_fluxes,
        "rms": std_r,
        "n_kept": int(kept.sum()),
        "n_rejected": int((~kept).sum()),
    }


def eval_continuum(fit, wl):
    return np.interp(wl, fit["knots"], fit["fluxes"], left=fit["fluxes"][0], right=fit["fluxes"][-1])


def main() -> int:
    print(f"Loading flat: {FLAT}")
    frame = np.array(Image.open(FLAT), dtype=np.float64)
    print(f"  shape={frame.shape}  max={frame.max():.0f}")
    with open(PROFILE, "rb") as f:
        profile = tomllib.load(f)
    orders = [o for o in profile["orders"] if o.get("enabled", True)]
    print(f"Profile: {len(orders)} orders")

    orders_data = []
    for o in orders:
        xs, flux, wl = extract_flat_flux(frame, o)
        # IN-ILLUMINATION mask: the synthesized-order domain is the full
        # detector width (0..2559) but the order only lights up a narrow
        # strip. Pixels off-strip aperture-sum to near-zero background
        # levels; including them drags the continuum fit to the noise
        # floor. Threshold at 10× the 10th-percentile flux of this order.
        noise_floor = np.percentile(flux[flux > 0], 10) if (flux > 0).any() else 0
        illum_mask = flux > 10 * noise_floor
        orders_data.append((xs, flux, wl, o, illum_mask))

    cfg_knots = 32  # fewer than 64 because only 5 orders here cover narrow λ range
    # Feed the fit only the in-illumination samples (where the order is
    # actually lit up). Off-strip aperture-sum zeros would otherwise
    # dominate the median-in-window knot estimate.
    fit = fit_dh3p_continuum(
        [(xs[m], f[m], w[m]) for xs, f, w, _, m in orders_data],
        n_knots=cfg_knots,
    )
    if fit is None:
        print("ERROR: continuum fit failed")
        return 1
    print(
        f"Continuum fit: {fit['n_kept']}/{fit['n_kept']+fit['n_rejected']} samples kept"
        f", residual std={fit['rms']:.1f}"
    )

    # Compute per-order blaze curves
    order_blazes = []
    for xs, flux, wl, o, illum_mask in orders_data:
        c_lamp = eval_continuum(fit, wl)
        raw_blaze = np.where(c_lamp > 1e-6, flux / c_lamp, 0.0)
        peak = np.nanmax(raw_blaze[np.isfinite(raw_blaze)])
        norm_blaze = raw_blaze / peak if peak > 0 else raw_blaze
        usable = norm_blaze >= 0.15
        order_blazes.append(
            dict(
                m=o.get("physical_order_number"),
                xs=xs,
                wl=wl,
                flux=flux,
                c_lamp=c_lamp,
                blaze=norm_blaze,
                usable=usable,
                raw_peak=peak,
            )
        )

    # Plot: 3 panels
    fig, axes = plt.subplots(3, 1, figsize=(14, 10))
    ax_flux, ax_cont, ax_blaze = axes

    # Top: raw extracted flat flux vs wavelength, with fitted C_lamp overlay
    for b in order_blazes:
        ax_flux.plot(b["wl"], b["flux"], linewidth=0.8, label=f"m={b['m']}")
    ax_flux.plot(fit["knots"], fit["fluxes"], "k--", linewidth=1.5, label=f"C_lamp ({cfg_knots} knots)")
    ax_flux.set_title(
        f"DH3P flat — per-order raw extracted flux + fitted lamp continuum\n"
        f"({fit['n_kept']} samples kept, {fit['n_rejected']} rejected by 3σ clip)"
    )
    ax_flux.set_xlabel("Wavelength (nm)")
    ax_flux.set_ylabel("Raw aperture-sum flux")
    ax_flux.legend(fontsize=8, ncol=3)
    ax_flux.grid(True, alpha=0.3)

    # Middle: same zoomed on continuum only
    ax_cont.plot(fit["knots"], fit["fluxes"], "k-o", markersize=4, linewidth=1)
    for b in order_blazes:
        ax_cont.plot(b["wl"], b["c_lamp"], alpha=0.7, linewidth=0.8)
    ax_cont.set_title("Lamp continuum C_lamp(λ) evaluated on each order's λ axis (colored) + knots (black)")
    ax_cont.set_xlabel("Wavelength (nm)")
    ax_cont.set_ylabel("C_lamp flux")
    ax_cont.grid(True, alpha=0.3)

    # Bottom: per-order blaze B_i(x) = F_i / C_lamp, peak-normalized, with 15% threshold
    for b in order_blazes:
        ax_blaze.plot(b["wl"], b["blaze"], linewidth=0.9, label=f"m={b['m']}")
        # Mark masked pixels
        masked_wl = b["wl"][~b["usable"]]
        if masked_wl.size > 0:
            ax_blaze.scatter(masked_wl, np.zeros_like(masked_wl) + 0.02, s=2, color="red", alpha=0.4)
    ax_blaze.axhline(0.15, color="red", linestyle="--", alpha=0.5, label="15% threshold")
    ax_blaze.set_title(
        "Per-order instrumental blaze B_i(x) = F_i(x) / C_lamp(λ), peak-normalised\n"
        "Red dots below = pixels masked out (blaze < 15% of per-order peak)"
    )
    ax_blaze.set_xlabel("Wavelength (nm)")
    ax_blaze.set_ylabel("B_i (normalised)")
    ax_blaze.set_ylim(-0.05, 1.15)
    ax_blaze.legend(fontsize=8, ncol=3)
    ax_blaze.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(OUT, dpi=140)
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
