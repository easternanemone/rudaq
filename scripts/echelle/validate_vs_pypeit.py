#!/usr/bin/env python3
"""
End-to-end validation of rust-daq echelle extraction vs PypeIt reference.

Compares per-order flux arrays and wavelength solutions between the rust-daq
echelle pipeline and PypeIt (or any spec1d FITS producer). Reports per-order
and aggregate statistics with SNR-stratified tolerances.

Usage:
    python validate_vs_pypeit.py --rust-json <rust_spectrum.json> --pypeit-fits <pypeit_spec1d.fits>
    python validate_vs_pypeit.py --rust-dir <rust_output_dir> --pypeit-dir <pypeit_output_dir>
    python validate_vs_pypeit.py --help

Exit codes:
    0  All tolerances pass.
    1  At least one bright-line (SNR > 100) tolerance failed.
    2  Bright lines passed but faint-line or wavelength tolerances failed.
"""

from __future__ import annotations

import argparse
import json
import sys
import textwrap
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Optional

import numpy as np

# ─── Acceptance tolerances ───────────────────────────────────────────────────
# These are embedded as constants so that the script is self-contained and
# the tolerances are version-controlled alongside the extraction code.

# Fractional RMS threshold for bright emission lines (SNR > 100).
# At high SNR the extraction should track the reference closely.
BRIGHT_LINE_RMS_FRAC_THRESHOLD = 0.05  # 5%

# Fractional RMS threshold for faint lines (SNR 10-100).
# Background subtraction noise and readout noise dominate at lower SNR.
FAINT_LINE_RMS_FRAC_THRESHOLD = 0.10  # 10%

# RMS wavelength-solution residual in nm. The Mechelle 5000 has resolving
# power R~5000 across 200-900 nm, so sub-pixel wavelength agreement at
# ~0.05 nm corresponds to ~0.1 px at 550 nm.
WAVELENGTH_RMS_NM_THRESHOLD = 0.05  # nm

# SNR boundaries
BRIGHT_SNR_MIN = 100.0
FAINT_SNR_MIN = 10.0
FAINT_SNR_MAX = 100.0


# ─── Data structures ────────────────────────────────────────────────────────

@dataclass
class OrderSpectrum:
    """A single extracted echelle order with wavelength and flux arrays."""
    order_index: int
    physical_order_number: Optional[int]
    wavelengths: np.ndarray  # nm
    flux: np.ndarray  # counts or counts/s
    wavelength_unit: str = "nm"

    def snr_estimate(self) -> float:
        """Rough per-order SNR from peak flux / noise-floor estimate."""
        if self.flux.size < 10:
            return 0.0
        # Noise ~ IQR of full flux distribution (IQR/1.349 ≈ sigma for Gaussian)
        sorted_flux = np.sort(self.flux)
        q25 = sorted_flux[len(sorted_flux) // 4]
        q75 = sorted_flux[len(sorted_flux) * 3 // 4]
        noise = (q75 - q25) / 1.349  # IQR → sigma for Gaussian
        peak = float(np.max(self.flux))
        if noise <= 0:
            return peak if peak > 0 else 0.0
        return peak / noise


@dataclass
class OrderComparison:
    """Comparison result for a single order."""
    order_index: int
    n_samples: int
    mean_ratio: float  # mean(rust / pypeit)
    rms_frac: float  # RMS of (rust - pypeit) / pypeit
    wavelength_rms_nm: float  # RMS wavelength offset
    snr_estimate: float  # from the pypeit (reference) spectrum
    classification: str  # "bright", "faint", or "below_threshold"
    passed: bool


@dataclass
class ValidationReport:
    """Aggregate validation report."""
    rust_source: str
    pypeit_source: str
    n_orders_compared: int
    n_orders_bright: int
    n_orders_faint: int
    bright_rms_frac_max: float
    faint_rms_frac_max: float
    wavelength_rms_nm_max: float
    bright_passed: bool
    faint_passed: bool
    wavelength_passed: bool
    all_passed: bool
    per_order: list = field(default_factory=list)
    tolerances: dict = field(default_factory=dict)


# ─── Loaders ─────────────────────────────────────────────────────────────────

def load_rust_json(path: Path) -> list[OrderSpectrum]:
    """Load rust-daq echelle extraction output (JSON).

    Accepts two formats:

    1. **Calibration profile** (``EchelleCalibrationProfile``): a JSON object
       with an ``"orders"`` array where each order has a ``"wavelength"``
       model (sampled or polynomial) and flux is not stored (returns empty
       flux; the caller should provide flux separately via the second
       format).

    2. **Extraction result** (``EchelleExtractionPreview`` serialised to
       JSON): a JSON object with an ``"orders"`` array where each order has
       ``"wavelengths"`` and ``"flux"`` arrays.

    Both shapes are emitted by different parts of the rust-daq pipeline;
    this loader normalises them into ``OrderSpectrum`` objects.
    """
    data = json.loads(path.read_text())
    orders = []

    # Normalise: the top-level object may contain "orders" directly,
    # or the entire file may be a list of order dicts.
    order_list = data if isinstance(data, list) else data.get("orders", [])

    def _to_nm(wl: np.ndarray, unit: str) -> np.ndarray:
        """Normalise wavelengths to nanometres."""
        u = unit.lower().strip()
        if u in ("aa", "angstrom", "angstroms", "a"):
            return wl / 10.0
        if u in ("um", "micron", "microns"):
            return wl * 1000.0
        # Heuristic: values > 2000 are likely Angstroms
        if u == "nm" and wl.size > 0 and float(np.max(wl)) > 2000.0:
            return wl / 10.0
        return wl  # assume nm

    for entry in order_list:
        # Format 2: extraction result with explicit arrays
        if "wavelengths" in entry and "flux" in entry:
            wl = np.asarray(entry["wavelengths"], dtype=np.float64)
            fl = np.asarray(entry["flux"], dtype=np.float64)
            oi = int(entry.get("relative_index", entry.get("order_index", 0)))
            phys = entry.get("physical_order_number")
            unit = entry.get("wavelength_unit", "nm")
            wl = _to_nm(wl, unit)
            orders.append(OrderSpectrum(
                order_index=oi,
                physical_order_number=int(phys) if phys is not None else None,
                wavelengths=wl,
                flux=fl,
                wavelength_unit="nm",
            ))
            continue

        # Format 1: calibration profile with wavelength model
        wl_model = entry.get("wavelength", {})
        model_type = wl_model.get("type", "")
        if model_type == "sampled":
            wl = np.asarray(wl_model.get("wavelengths", []), dtype=np.float64)
        elif model_type == "polynomial":
            coeffs = wl_model.get("coefficients", [])
            domain_start = wl_model.get("domain_start", 0.0)
            domain_end = wl_model.get("domain_end", 1.0)
            basis = wl_model.get("basis", "chebyshev")
            n_px = int(entry.get("sample_end", 2047)) - int(entry.get("sample_start", 0)) + 1
            pixels = np.linspace(domain_start, domain_end, n_px)
            wl = _eval_polynomial(coeffs, pixels, basis, domain_start, domain_end)
        else:
            continue  # skip orders without wavelength info

        fl_raw = entry.get("flux", entry.get("flux_bgsub", []))
        fl = np.asarray(fl_raw, dtype=np.float64) if fl_raw else np.zeros(len(wl))

        oi = int(entry.get("relative_index", entry.get("order_index", 0)))
        phys = entry.get("physical_order_number")
        unit = wl_model.get("unit", "nm")
        wl = _to_nm(wl, unit)
        orders.append(OrderSpectrum(
            order_index=oi,
            physical_order_number=int(phys) if phys is not None else None,
            wavelengths=wl,
            flux=fl,
            wavelength_unit="nm",
        ))

    return orders


def _eval_polynomial(
    coeffs: list[float],
    x: np.ndarray,
    basis: str,
    domain_start: float,
    domain_end: float,
) -> np.ndarray:
    """Evaluate a polynomial wavelength model (Chebyshev or monomial)."""
    if basis == "chebyshev":
        # Map x to [-1, 1] for Chebyshev evaluation
        x_mapped = 2.0 * (x - domain_start) / (domain_end - domain_start) - 1.0
        return np.polynomial.chebyshev.chebval(x_mapped, coeffs)
    else:
        # Monomial: c0 + c1*x + c2*x^2 + ...
        return np.polyval(coeffs[::-1], x)


def load_pypeit_fits(path: Path) -> list[OrderSpectrum]:
    """Load a PypeIt spec1d FITS file.

    PypeIt writes one binary-table HDU per extracted object/order with
    columns: ``OPT_WAVE`` (or ``BOX_WAVE``), ``OPT_COUNTS`` (or
    ``BOX_COUNTS``), ``OPT_COUNTS_IVAR``, etc.

    We prefer the optimal-extraction columns when available.
    """
    try:
        from astropy.io import fits  # type: ignore
    except ImportError:
        print(
            "ERROR: astropy is required to read PypeIt FITS files.\n"
            "  Install: pip install astropy",
            file=sys.stderr,
        )
        sys.exit(1)

    orders = []
    with fits.open(str(path)) as hdul:
        for i, hdu in enumerate(hdul):
            if not hasattr(hdu, "columns") or hdu.columns is None:
                continue
            col_names = [c.name.upper() for c in hdu.columns]

            # Prefer optimal extraction, fall back to boxcar
            if "OPT_WAVE" in col_names:
                wl = np.asarray(hdu.data["OPT_WAVE"], dtype=np.float64)
                flux_key = "OPT_COUNTS" if "OPT_COUNTS" in col_names else "OPT_FLAM"
                fl = np.asarray(hdu.data[flux_key], dtype=np.float64)
            elif "BOX_WAVE" in col_names:
                wl = np.asarray(hdu.data["BOX_WAVE"], dtype=np.float64)
                flux_key = "BOX_COUNTS" if "BOX_COUNTS" in col_names else "BOX_FLAM"
                fl = np.asarray(hdu.data[flux_key], dtype=np.float64)
            else:
                continue

            # PypeIt wavelengths are in Angstroms; convert to nm
            if wl.max() > 2000.0:
                wl = wl / 10.0

            # Extract order number from HDU name if available
            # PypeIt names HDUs like "SPAT0123-SLIT0001-DET01"
            hdu_name = hdu.name if hasattr(hdu, "name") else ""
            order_idx = i - 1  # Skip primary HDU
            phys_order = None
            if "ORDER" in hdu_name.upper():
                try:
                    phys_order = int(hdu_name.split("ORDER")[-1].split("-")[0])
                except (ValueError, IndexError):
                    pass

            # Filter to valid data (non-zero wavelength)
            valid = wl > 0
            orders.append(OrderSpectrum(
                order_index=order_idx,
                physical_order_number=phys_order,
                wavelengths=wl[valid],
                flux=fl[valid],
            ))

    return orders


def load_pypeit_dir(dirpath: Path) -> list[OrderSpectrum]:
    """Load all spec1d FITS files from a PypeIt output directory."""
    all_orders = []
    fits_files = sorted(dirpath.glob("spec1d_*.fits"))
    if not fits_files:
        fits_files = sorted(dirpath.glob("*.fits"))
    for f in fits_files:
        all_orders.extend(load_pypeit_fits(f))
    return all_orders


def load_rust_dir(dirpath: Path) -> list[OrderSpectrum]:
    """Load all JSON extraction outputs from a rust-daq output directory."""
    all_orders = []
    json_files = sorted(dirpath.glob("*.json"))
    for f in json_files:
        try:
            all_orders.extend(load_rust_json(f))
        except (json.JSONDecodeError, KeyError):
            continue
    return all_orders


# ─── Comparison engine ───────────────────────────────────────────────────────

def resample_to_common_grid(
    wl_a: np.ndarray,
    flux_a: np.ndarray,
    wl_b: np.ndarray,
    flux_b: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Resample both spectra onto a common wavelength grid.

    Uses linear interpolation on the intersection of wavelength ranges.
    Returns (common_wavelengths, flux_a_resampled, flux_b_resampled).
    """
    wl_min = max(float(wl_a.min()), float(wl_b.min()))
    wl_max = min(float(wl_a.max()), float(wl_b.max()))

    if wl_max <= wl_min:
        return np.array([]), np.array([]), np.array([])

    # Use the finer grid spacing
    step_a = np.median(np.diff(wl_a)) if len(wl_a) > 1 else 1.0
    step_b = np.median(np.diff(wl_b)) if len(wl_b) > 1 else 1.0
    step = min(abs(float(step_a)), abs(float(step_b)))
    if step <= 0:
        step = 0.01  # fallback

    common_wl = np.arange(wl_min, wl_max, step)
    if common_wl.size == 0:
        return np.array([]), np.array([]), np.array([])

    flux_a_interp = np.interp(common_wl, wl_a, flux_a)
    flux_b_interp = np.interp(common_wl, wl_b, flux_b)

    return common_wl, flux_a_interp, flux_b_interp


def match_orders(
    rust_orders: list[OrderSpectrum],
    pypeit_orders: list[OrderSpectrum],
) -> list[tuple[OrderSpectrum, OrderSpectrum]]:
    """Match rust-daq orders to PypeIt orders by wavelength overlap.

    First tries to match by physical order number if available, then falls
    back to maximum wavelength overlap.
    """
    matched = []
    used_pypeit = set()

    # Pass 1: match by physical order number
    pypeit_by_phys = {}
    for po in pypeit_orders:
        if po.physical_order_number is not None:
            pypeit_by_phys[po.physical_order_number] = po

    for ro in rust_orders:
        if ro.physical_order_number is not None and ro.physical_order_number in pypeit_by_phys:
            po = pypeit_by_phys[ro.physical_order_number]
            if id(po) not in used_pypeit:
                matched.append((ro, po))
                used_pypeit.add(id(po))

    # Pass 2: match remaining by wavelength overlap
    matched_rust_ids = {id(m[0]) for m in matched}
    unmatched_rust = [ro for ro in rust_orders if id(ro) not in matched_rust_ids]
    unmatched_pypeit = [po for po in pypeit_orders if id(po) not in used_pypeit]

    for ro in unmatched_rust:
        if ro.wavelengths.size == 0:
            continue
        best_overlap = 0.0
        best_po = None
        ro_min, ro_max = float(ro.wavelengths.min()), float(ro.wavelengths.max())
        for po in unmatched_pypeit:
            if po.wavelengths.size == 0 or id(po) in used_pypeit:
                continue
            po_min, po_max = float(po.wavelengths.min()), float(po.wavelengths.max())
            overlap = max(0.0, min(ro_max, po_max) - max(ro_min, po_min))
            if overlap > best_overlap:
                best_overlap = overlap
                best_po = po
        if best_po is not None and best_overlap > 0:
            matched.append((ro, best_po))
            used_pypeit.add(id(best_po))

    return matched


def compare_order(
    rust: OrderSpectrum,
    pypeit: OrderSpectrum,
) -> OrderComparison:
    """Compare a single order between rust-daq and PypeIt."""
    common_wl, rust_flux, pypeit_flux = resample_to_common_grid(
        rust.wavelengths, rust.flux,
        pypeit.wavelengths, pypeit.flux,
    )

    n_samples = common_wl.size
    if n_samples == 0:
        return OrderComparison(
            order_index=rust.order_index,
            n_samples=0,
            mean_ratio=float("nan"),
            rms_frac=float("nan"),
            wavelength_rms_nm=float("nan"),
            snr_estimate=0.0,
            classification="below_threshold",
            passed=True,  # no data to fail
        )

    # Flux comparison: fractional residual where PypeIt flux is significant
    # Avoid division by near-zero flux values
    ref_abs = np.abs(pypeit_flux)
    significant = ref_abs > np.percentile(ref_abs, 10)  # bottom 10% excluded

    if np.count_nonzero(significant) < 5:
        mean_ratio = float("nan")
        rms_frac = float("nan")
    else:
        ratio = rust_flux[significant] / pypeit_flux[significant]
        mean_ratio = float(np.mean(ratio))
        frac_residual = (rust_flux[significant] - pypeit_flux[significant]) / pypeit_flux[significant]
        rms_frac = float(np.sqrt(np.mean(frac_residual ** 2)))

    # Wavelength offset: RMS of the wavelength shift between peaks
    # For an overall check, compare centroids of strong features
    wavelength_rms_nm = _estimate_wavelength_offset(
        common_wl, rust_flux, pypeit_flux,
    )

    snr = pypeit.snr_estimate()

    if snr >= BRIGHT_SNR_MIN:
        classification = "bright"
        threshold = BRIGHT_LINE_RMS_FRAC_THRESHOLD
    elif snr >= FAINT_SNR_MIN:
        classification = "faint"
        threshold = FAINT_LINE_RMS_FRAC_THRESHOLD
    else:
        classification = "below_threshold"
        threshold = float("inf")

    flux_passed = np.isnan(rms_frac) or rms_frac < threshold
    wl_passed = np.isnan(wavelength_rms_nm) or wavelength_rms_nm < WAVELENGTH_RMS_NM_THRESHOLD

    return OrderComparison(
        order_index=rust.order_index,
        n_samples=n_samples,
        mean_ratio=mean_ratio,
        rms_frac=rms_frac,
        wavelength_rms_nm=wavelength_rms_nm,
        snr_estimate=snr,
        classification=classification,
        passed=bool(flux_passed and wl_passed),
    )


def _estimate_wavelength_offset(
    common_wl: np.ndarray,
    flux_a: np.ndarray,
    flux_b: np.ndarray,
) -> float:
    """Estimate wavelength offset via cross-correlation of strong features.

    Returns RMS offset in nm. If the spectra are too featureless to
    estimate an offset, returns 0.0.
    """
    if common_wl.size < 20:
        return 0.0

    try:
        from scipy.signal import correlate  # type: ignore
    except ImportError:
        # Fallback: simple peak-matching approach
        return _peak_based_wavelength_offset(common_wl, flux_a, flux_b)

    # Normalise both to zero-mean unit-variance
    a_norm = flux_a - np.mean(flux_a)
    b_norm = flux_b - np.mean(flux_b)
    a_std = np.std(a_norm)
    b_std = np.std(b_norm)
    if a_std <= 0 or b_std <= 0:
        return 0.0
    a_norm /= a_std
    b_norm /= b_std

    # Cross-correlate within a +/- 50 pixel window
    max_lag = min(50, common_wl.size // 4)
    cc = correlate(a_norm, b_norm, mode="full")
    mid = len(cc) // 2
    cc_window = cc[mid - max_lag : mid + max_lag + 1]

    peak_idx = int(np.argmax(cc_window)) - max_lag
    step = float(np.median(np.diff(common_wl)))
    offset_nm = abs(peak_idx * step)

    return offset_nm


def _peak_based_wavelength_offset(
    common_wl: np.ndarray,
    flux_a: np.ndarray,
    flux_b: np.ndarray,
) -> float:
    """Fallback wavelength offset via matching the N brightest peaks."""
    n_peaks = 5
    step = float(np.median(np.diff(common_wl))) if len(common_wl) > 1 else 1.0

    def find_peaks(flux: np.ndarray, n: int) -> list[int]:
        if flux.size < 3:
            return []
        # Simple local-max detection
        candidates = []
        for i in range(1, len(flux) - 1):
            if flux[i] > flux[i - 1] and flux[i] > flux[i + 1]:
                candidates.append((flux[i], i))
        candidates.sort(reverse=True)
        # Enforce minimum separation of 10 pixels
        selected = []
        for _, idx in candidates:
            if all(abs(idx - s) >= 10 for s in selected):
                selected.append(idx)
            if len(selected) >= n:
                break
        return sorted(selected)

    peaks_a = find_peaks(flux_a, n_peaks)
    peaks_b = find_peaks(flux_b, n_peaks)

    if len(peaks_a) < 2 or len(peaks_b) < 2:
        return 0.0

    # Match peaks by proximity and compute offsets
    offsets = []
    used = set()
    for pa in peaks_a:
        best_dist = float("inf")
        best_pb = None
        for pb in peaks_b:
            if pb in used:
                continue
            d = abs(pa - pb)
            if d < best_dist:
                best_dist = d
                best_pb = pb
        if best_pb is not None and best_dist < 20:
            offsets.append((pa - best_pb) * step)
            used.add(best_pb)

    if not offsets:
        return 0.0
    return float(np.sqrt(np.mean(np.array(offsets) ** 2)))


# ─── Report generation ───────────────────────────────────────────────────────

def build_report(
    rust_source: str,
    pypeit_source: str,
    comparisons: list[OrderComparison],
) -> ValidationReport:
    """Build aggregate validation report from per-order comparisons."""
    bright = [c for c in comparisons if c.classification == "bright"]
    faint = [c for c in comparisons if c.classification == "faint"]

    bright_rms_max = max((c.rms_frac for c in bright if not np.isnan(c.rms_frac)), default=0.0)
    faint_rms_max = max((c.rms_frac for c in faint if not np.isnan(c.rms_frac)), default=0.0)
    wl_rms_max = max(
        (c.wavelength_rms_nm for c in comparisons if not np.isnan(c.wavelength_rms_nm)),
        default=0.0,
    )

    bright_passed = all(c.passed for c in bright)
    faint_passed = all(c.passed for c in faint)
    wavelength_passed = wl_rms_max < WAVELENGTH_RMS_NM_THRESHOLD

    return ValidationReport(
        rust_source=rust_source,
        pypeit_source=pypeit_source,
        n_orders_compared=len(comparisons),
        n_orders_bright=len(bright),
        n_orders_faint=len(faint),
        bright_rms_frac_max=bright_rms_max,
        faint_rms_frac_max=faint_rms_max,
        wavelength_rms_nm_max=wl_rms_max,
        bright_passed=bright_passed,
        faint_passed=faint_passed,
        wavelength_passed=wavelength_passed,
        all_passed=bright_passed and faint_passed and wavelength_passed,
        per_order=[asdict(c) for c in comparisons],
        tolerances={
            "bright_rms_frac_threshold": BRIGHT_LINE_RMS_FRAC_THRESHOLD,
            "faint_rms_frac_threshold": FAINT_LINE_RMS_FRAC_THRESHOLD,
            "wavelength_rms_nm_threshold": WAVELENGTH_RMS_NM_THRESHOLD,
            "bright_snr_min": BRIGHT_SNR_MIN,
            "faint_snr_range": [FAINT_SNR_MIN, FAINT_SNR_MAX],
        },
    )


def print_table(report: ValidationReport) -> None:
    """Print a human-readable comparison table to stdout."""
    header = (
        f"{'Order':>6} {'SNR':>8} {'Class':>10} {'Samples':>8} "
        f"{'MeanRatio':>10} {'RMS%':>8} {'WL_RMS_nm':>10} {'Status':>8}"
    )
    print()
    print("=" * len(header))
    print("Echelle Extraction Validation: rust-daq vs PypeIt")
    print("=" * len(header))
    print(f"  Rust source:   {report.rust_source}")
    print(f"  PypeIt source: {report.pypeit_source}")
    print(f"  Orders compared: {report.n_orders_compared}")
    print()
    print(header)
    print("-" * len(header))

    for order in report.per_order:
        snr_str = f"{order['snr_estimate']:.1f}" if not np.isnan(order["snr_estimate"]) else "N/A"
        ratio_str = f"{order['mean_ratio']:.4f}" if not np.isnan(order["mean_ratio"]) else "N/A"
        rms_str = f"{order['rms_frac'] * 100:.2f}" if not np.isnan(order["rms_frac"]) else "N/A"
        wl_str = f"{order['wavelength_rms_nm']:.4f}" if not np.isnan(order["wavelength_rms_nm"]) else "N/A"
        status = "PASS" if order["passed"] else "FAIL"
        print(
            f"{order['order_index']:>6} {snr_str:>8} {order['classification']:>10} "
            f"{order['n_samples']:>8} {ratio_str:>10} {rms_str:>8} {wl_str:>10} {status:>8}"
        )

    print("-" * len(header))
    print()
    print("Aggregate Results:")
    print(f"  Bright lines (SNR > {BRIGHT_SNR_MIN:.0f}): "
          f"max RMS = {report.bright_rms_frac_max * 100:.2f}% "
          f"(threshold: {BRIGHT_LINE_RMS_FRAC_THRESHOLD * 100:.0f}%) "
          f"{'PASS' if report.bright_passed else 'FAIL'}")
    print(f"  Faint lines  (SNR {FAINT_SNR_MIN:.0f}-{FAINT_SNR_MAX:.0f}): "
          f"max RMS = {report.faint_rms_frac_max * 100:.2f}% "
          f"(threshold: {FAINT_LINE_RMS_FRAC_THRESHOLD * 100:.0f}%) "
          f"{'PASS' if report.faint_passed else 'FAIL'}")
    print(f"  Wavelength solution: "
          f"max RMS = {report.wavelength_rms_nm_max:.4f} nm "
          f"(threshold: {WAVELENGTH_RMS_NM_THRESHOLD} nm) "
          f"{'PASS' if report.wavelength_passed else 'FAIL'}")
    print()
    overall = "ALL PASSED" if report.all_passed else "FAILED"
    print(f"  Overall: {overall}")
    print("=" * len(header))
    print()


def generate_html_report(report: ValidationReport, out_path: Path) -> None:
    """Generate an HTML report with comparison plots.

    Requires matplotlib. If matplotlib is not available, writes a
    plots-free HTML table instead.
    """
    try:
        import matplotlib  # type: ignore
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt  # type: ignore
        has_mpl = True
    except ImportError:
        has_mpl = False

    import base64
    from io import BytesIO

    plot_images = []
    if has_mpl:
        # Per-order RMS bar chart
        fig, axes = plt.subplots(1, 2, figsize=(14, 5))

        # Left: RMS fractional difference per order
        ax = axes[0]
        oi = [o["order_index"] for o in report.per_order]
        rms = [o["rms_frac"] * 100 if not np.isnan(o["rms_frac"]) else 0 for o in report.per_order]
        colors = []
        for o in report.per_order:
            if o["classification"] == "bright":
                colors.append("#2196F3" if o["passed"] else "#F44336")
            elif o["classification"] == "faint":
                colors.append("#4CAF50" if o["passed"] else "#FF9800")
            else:
                colors.append("#9E9E9E")
        ax.bar(range(len(oi)), rms, color=colors, edgecolor="black", linewidth=0.5)
        ax.axhline(y=BRIGHT_LINE_RMS_FRAC_THRESHOLD * 100, color="blue", linestyle="--",
                    label=f"Bright threshold ({BRIGHT_LINE_RMS_FRAC_THRESHOLD * 100:.0f}%)")
        ax.axhline(y=FAINT_LINE_RMS_FRAC_THRESHOLD * 100, color="green", linestyle="--",
                    label=f"Faint threshold ({FAINT_LINE_RMS_FRAC_THRESHOLD * 100:.0f}%)")
        ax.set_xlabel("Order Index")
        ax.set_ylabel("RMS Fractional Difference (%)")
        ax.set_title("Flux RMS by Order")
        ax.legend(fontsize=8)
        ax.set_xticks(range(len(oi)))
        ax.set_xticklabels(oi, fontsize=7)

        # Right: Wavelength RMS per order
        ax = axes[1]
        wl_rms = [
            o["wavelength_rms_nm"] if not np.isnan(o["wavelength_rms_nm"]) else 0
            for o in report.per_order
        ]
        ax.bar(range(len(oi)), wl_rms, color="#FF9800", edgecolor="black", linewidth=0.5)
        ax.axhline(y=WAVELENGTH_RMS_NM_THRESHOLD, color="red", linestyle="--",
                    label=f"Threshold ({WAVELENGTH_RMS_NM_THRESHOLD} nm)")
        ax.set_xlabel("Order Index")
        ax.set_ylabel("Wavelength RMS (nm)")
        ax.set_title("Wavelength Solution RMS by Order")
        ax.legend(fontsize=8)
        ax.set_xticks(range(len(oi)))
        ax.set_xticklabels(oi, fontsize=7)

        plt.tight_layout()
        buf = BytesIO()
        fig.savefig(buf, format="png", dpi=120)
        plt.close(fig)
        buf.seek(0)
        plot_images.append(base64.b64encode(buf.read()).decode("ascii"))

    # Build HTML
    status_color = "#4CAF50" if report.all_passed else "#F44336"
    status_text = "ALL PASSED" if report.all_passed else "FAILED"

    rows_html = ""
    for o in report.per_order:
        snr_s = f"{o['snr_estimate']:.1f}" if not np.isnan(o["snr_estimate"]) else "N/A"
        ratio_s = f"{o['mean_ratio']:.4f}" if not np.isnan(o["mean_ratio"]) else "N/A"
        rms_s = f"{o['rms_frac'] * 100:.2f}%" if not np.isnan(o["rms_frac"]) else "N/A"
        wl_s = f"{o['wavelength_rms_nm']:.4f}" if not np.isnan(o["wavelength_rms_nm"]) else "N/A"
        row_color = "#e8f5e9" if o["passed"] else "#ffebee"
        rows_html += (
            f'<tr style="background:{row_color}">'
            f"<td>{o['order_index']}</td><td>{snr_s}</td><td>{o['classification']}</td>"
            f"<td>{o['n_samples']}</td><td>{ratio_s}</td><td>{rms_s}</td>"
            f"<td>{wl_s}</td><td>{'PASS' if o['passed'] else 'FAIL'}</td></tr>\n"
        )

    plots_html = ""
    for img_b64 in plot_images:
        plots_html += f'<img src="data:image/png;base64,{img_b64}" style="max-width:100%">\n'

    html = textwrap.dedent(f"""\
    <!DOCTYPE html>
    <html>
    <head>
    <meta charset="utf-8">
    <title>Echelle Validation Report</title>
    <style>
      body {{ font-family: sans-serif; max-width: 1100px; margin: 20px auto; }}
      table {{ border-collapse: collapse; width: 100%; margin: 16px 0; }}
      th, td {{ border: 1px solid #ccc; padding: 6px 10px; text-align: right; }}
      th {{ background: #f5f5f5; }}
      .status {{ font-size: 1.5em; font-weight: bold; color: {status_color}; }}
    </style>
    </head>
    <body>
    <h1>Echelle Extraction Validation: rust-daq vs PypeIt</h1>
    <p class="status">Overall: {status_text}</p>
    <p>Rust source: <code>{report.rust_source}</code></p>
    <p>PypeIt source: <code>{report.pypeit_source}</code></p>
    <h2>Summary</h2>
    <ul>
      <li>Orders compared: {report.n_orders_compared}</li>
      <li>Bright (SNR &gt; {BRIGHT_SNR_MIN:.0f}): {report.n_orders_bright} orders,
          max RMS = {report.bright_rms_frac_max * 100:.2f}%
          (threshold: {BRIGHT_LINE_RMS_FRAC_THRESHOLD * 100:.0f}%)
          &mdash; {'PASS' if report.bright_passed else 'FAIL'}</li>
      <li>Faint (SNR {FAINT_SNR_MIN:.0f}&ndash;{FAINT_SNR_MAX:.0f}): {report.n_orders_faint} orders,
          max RMS = {report.faint_rms_frac_max * 100:.2f}%
          (threshold: {FAINT_LINE_RMS_FRAC_THRESHOLD * 100:.0f}%)
          &mdash; {'PASS' if report.faint_passed else 'FAIL'}</li>
      <li>Wavelength solution: max RMS = {report.wavelength_rms_nm_max:.4f} nm
          (threshold: {WAVELENGTH_RMS_NM_THRESHOLD} nm)
          &mdash; {'PASS' if report.wavelength_passed else 'FAIL'}</li>
    </ul>
    {plots_html}
    <h2>Per-Order Results</h2>
    <table>
    <tr><th>Order</th><th>SNR</th><th>Class</th><th>Samples</th>
        <th>Mean Ratio</th><th>RMS %</th><th>WL RMS (nm)</th><th>Status</th></tr>
    {rows_html}
    </table>
    <h2>Tolerances</h2>
    <pre>{json.dumps(report.tolerances, indent=2)}</pre>
    </body>
    </html>
    """)

    out_path.write_text(html)
    print(f"HTML report written to {out_path}")


# ─── CLI ─────────────────────────────────────────────────────────────────────

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate rust-daq echelle extraction against PypeIt reference output.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent("""\
            Exit codes:
              0  All tolerances pass.
              1  At least one bright-line (SNR > 100) tolerance failed.
              2  Only faint-line (SNR 10-100) tolerances failed.

            Examples:
              # Compare single files
              python validate_vs_pypeit.py \\
                  --rust-json extraction.json \\
                  --pypeit-fits spec1d_mechelle.fits

              # Compare directories
              python validate_vs_pypeit.py \\
                  --rust-dir ./rust_output/ \\
                  --pypeit-dir ./pypeit_output/

              # Generate HTML report with plots
              python validate_vs_pypeit.py \\
                  --rust-json extraction.json \\
                  --pypeit-fits spec1d_mechelle.fits \\
                  --generate-report report.html

            Tolerances (embedded in script):
              Bright lines (SNR > 100): rms_frac < 5%%
              Faint lines  (SNR 10-100): rms_frac < 10%%
              Wavelength solution: < 0.05 nm RMS
        """),
    )

    # Input sources
    input_group = parser.add_argument_group("Input sources")
    input_group.add_argument(
        "--rust-json",
        type=Path,
        help="Path to a single rust-daq JSON extraction output file.",
    )
    input_group.add_argument(
        "--rust-dir",
        type=Path,
        help="Directory containing rust-daq JSON extraction output files.",
    )
    input_group.add_argument(
        "--pypeit-fits",
        type=Path,
        help="Path to a single PypeIt spec1d FITS file.",
    )
    input_group.add_argument(
        "--pypeit-dir",
        type=Path,
        help="Directory containing PypeIt spec1d FITS files.",
    )

    # Output options
    output_group = parser.add_argument_group("Output options")
    output_group.add_argument(
        "--generate-report",
        type=Path,
        metavar="PATH",
        help="Generate an HTML report with comparison plots at the given path.",
    )
    output_group.add_argument(
        "--json",
        action="store_true",
        help="Print the full report as JSON to stdout (in addition to the table).",
    )
    output_group.add_argument(
        "--quiet",
        action="store_true",
        help="Suppress the per-order table; only print summary and exit code.",
    )

    return parser.parse_args()


def main() -> int:
    args = parse_args()

    # ── Load rust-daq output ──────────────────────────────────────────
    if args.rust_json:
        if not args.rust_json.exists():
            print(f"ERROR: rust-daq JSON file not found: {args.rust_json}", file=sys.stderr)
            return 1
        rust_orders = load_rust_json(args.rust_json)
        rust_source = str(args.rust_json)
    elif args.rust_dir:
        if not args.rust_dir.is_dir():
            print(f"ERROR: rust-daq directory not found: {args.rust_dir}", file=sys.stderr)
            return 1
        rust_orders = load_rust_dir(args.rust_dir)
        rust_source = str(args.rust_dir)
    else:
        print("ERROR: specify --rust-json or --rust-dir", file=sys.stderr)
        return 1

    # ── Load PypeIt output ────────────────────────────────────────────
    if args.pypeit_fits:
        if not args.pypeit_fits.exists():
            print(f"ERROR: PypeIt FITS file not found: {args.pypeit_fits}", file=sys.stderr)
            return 1
        pypeit_orders = load_pypeit_fits(args.pypeit_fits)
        pypeit_source = str(args.pypeit_fits)
    elif args.pypeit_dir:
        if not args.pypeit_dir.is_dir():
            print(f"ERROR: PypeIt directory not found: {args.pypeit_dir}", file=sys.stderr)
            return 1
        pypeit_orders = load_pypeit_dir(args.pypeit_dir)
        pypeit_source = str(args.pypeit_dir)
    else:
        print("ERROR: specify --pypeit-fits or --pypeit-dir", file=sys.stderr)
        return 1

    # ── Validate inputs ───────────────────────────────────────────────
    if not rust_orders:
        print("ERROR: no orders loaded from rust-daq output.", file=sys.stderr)
        return 1
    if not pypeit_orders:
        print("ERROR: no orders loaded from PypeIt output.", file=sys.stderr)
        return 1

    print(f"Loaded {len(rust_orders)} rust-daq orders, {len(pypeit_orders)} PypeIt orders.")

    # ── Match and compare ─────────────────────────────────────────────
    matched = match_orders(rust_orders, pypeit_orders)
    if not matched:
        print("ERROR: no orders could be matched between the two pipelines.", file=sys.stderr)
        return 1

    print(f"Matched {len(matched)} order pairs.")

    comparisons = [compare_order(r, p) for r, p in matched]
    report = build_report(rust_source, pypeit_source, comparisons)

    # ── Output ────────────────────────────────────────────────────────
    if not args.quiet:
        print_table(report)

    if args.json:
        print(json.dumps(asdict(report), indent=2, default=_json_default))

    if args.generate_report:
        generate_html_report(report, args.generate_report)

    # ── Exit code ─────────────────────────────────────────────────────
    if report.all_passed:
        return 0
    elif not report.bright_passed:
        return 1
    else:
        # Bright lines passed but faint-line or wavelength tolerances failed
        return 2


def _json_default(obj: object) -> object:
    """JSON serializer fallback for numpy types."""
    if isinstance(obj, (np.integer,)):
        return int(obj)
    if isinstance(obj, (np.floating,)):
        return float(obj)
    if isinstance(obj, np.ndarray):
        return obj.tolist()
    if isinstance(obj, float) and np.isnan(obj):
        return None
    raise TypeError(f"Object of type {type(obj)} is not JSON serializable")


if __name__ == "__main__":
    raise SystemExit(main())
