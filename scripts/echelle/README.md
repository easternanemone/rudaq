# Echelle Extraction Validation Suite

End-to-end science-grade comparison of the rust-daq echelle extraction pipeline
against a PypeIt (or equivalent) reference reduction.

## Purpose

This suite validates that rust-daq's built-in echelle extraction produces
spectra consistent with established astronomical spectroscopy software. It
compares per-order wavelength solutions and extracted flux arrays, categorises
results by signal-to-noise ratio, and enforces acceptance tolerances suitable
for science-grade data.

## Prerequisites

**Python 3.10+** with the following packages:

```
numpy
astropy          # for reading PypeIt FITS output
scipy            # optional, improves wavelength-offset estimation
matplotlib       # optional, needed for --generate-report plots
```

Install:
```bash
pip install numpy astropy scipy matplotlib
```

**PypeIt** (for generating the reference reduction):
```bash
pip install pypeit
```

## Directory contents

| File | Description |
|------|-------------|
| `validate_vs_pypeit.py` | Main validation script (executable) |
| `mechelle_pypeit_template.pypeit` | PypeIt configuration template for Mechelle 5000 |
| `reference_extract_hg2.py` | Python reference extractor for canned HG-2 frames |
| `fixture_sidecar_hg2.py` | Fixture-backed sidecar for offline testing |
| `ci_validation_step.yml` | Reusable CI workflow snippet |
| `README.md` | This file |

## Workflow

### 1. Run rust-daq extraction

Extract spectra from an echelle frame using the rust-daq pipeline. The output
is a JSON file with per-order wavelength and flux arrays:

```bash
# Using the rust-daq daemon + gRPC client:
daq-client extract-echelle \
    --frame arc_HgAr.bin \
    --profile mechelle5000_profile.json \
    --output rust_extraction.json

# Or via the UI: open the image viewer, load a calibration profile,
# and use "Export Extraction" to save the JSON.
```

The JSON output follows the `EchelleExtractionPreview` format:
```json
{
  "orders": [
    {
      "relative_index": 0,
      "physical_order_number": 87,
      "wavelength_unit": "nm",
      "wavelengths": [200.1, 200.2, ...],
      "flux": [12.5, 13.1, ...]
    }
  ]
}
```

### 2. Run PypeIt reduction

Convert raw frames to FITS (if needed) and reduce with PypeIt:

```bash
# Convert rust-daq binary frames to FITS
python reference_extract_hg2.py --dataset-dir /path/to/capture/

# Set up PypeIt (copy and edit the template)
cp mechelle_pypeit_template.pypeit mechelle5000.pypeit
# Edit mechelle5000.pypeit: update [data] section with your FITS paths

# Run PypeIt reduction
run_pypeit mechelle5000.pypeit
```

PypeIt outputs `spec1d_*.fits` files in the `Science/` subdirectory.

### 3. Run the comparison

```bash
# Compare single files
python validate_vs_pypeit.py \
    --rust-json rust_extraction.json \
    --pypeit-fits Science/spec1d_mechelle.fits

# Compare directories (multiple frames)
python validate_vs_pypeit.py \
    --rust-dir ./rust_output/ \
    --pypeit-dir ./Science/

# Generate an HTML report with plots
python validate_vs_pypeit.py \
    --rust-json rust_extraction.json \
    --pypeit-fits Science/spec1d_mechelle.fits \
    --generate-report validation_report.html

# JSON output for CI consumption
python validate_vs_pypeit.py \
    --rust-json rust_extraction.json \
    --pypeit-fits Science/spec1d_mechelle.fits \
    --json --quiet
```

## Acceptance tolerances

| Category | SNR range | Metric | Threshold | Rationale |
|----------|-----------|--------|-----------|-----------|
| Bright lines | SNR > 100 | Fractional RMS | < 5% | At high SNR, extraction should closely track the reference. Dominated by trace accuracy and aperture definition. |
| Faint lines | SNR 10-100 | Fractional RMS | < 10% | Background subtraction noise and readout noise dominate. Wider tolerance accounts for different background estimation methods. |
| Wavelength | All orders | RMS offset | < 0.05 nm | Sub-pixel agreement at R~5000. Corresponds to ~0.1 px at 550 nm on the Mechelle 5000. |

These tolerances are embedded as constants at the top of `validate_vs_pypeit.py`
and are version-controlled alongside the extraction code.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | All tolerances pass |
| 1 | At least one bright-line tolerance failed (science-critical) |
| 2 | Only faint-line tolerances failed (bright lines passed) |

The distinction allows CI to enforce bright-line accuracy strictly while
treating faint-line deviations as warnings during initial development.

## Example output

```
======================================================================
Echelle Extraction Validation: rust-daq vs PypeIt
======================================================================
  Rust source:   rust_extraction.json
  PypeIt source: spec1d_mechelle.fits
  Orders compared: 22

 Order      SNR      Class  Samples  MeanRatio    RMS%  WL_RMS_nm   Status
----------------------------------------------------------------------
     0    245.3     bright     2048     1.0012    1.23     0.0082     PASS
     1    312.7     bright     2048     0.9998    0.89     0.0071     PASS
     2     87.1      faint     2048     1.0034    3.45     0.0123     PASS
    ...
----------------------------------------------------------------------

Aggregate Results:
  Bright lines (SNR > 100): max RMS = 2.31% (threshold: 5%) PASS
  Faint lines  (SNR 10-100): max RMS = 7.82% (threshold: 10%) PASS
  Wavelength solution: max RMS = 0.0182 nm (threshold: 0.05 nm) PASS

  Overall: ALL PASSED
======================================================================
```
