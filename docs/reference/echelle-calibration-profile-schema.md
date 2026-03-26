# Echelle Calibration Profile Schema (v1)

This document defines the canonical calibration profile format used by rust-daq
to convert a 2D echellegram (for example, Andor iSTAR sCMOS + Mechelle 5000)
into one or more 1D spectra.

The Rust source of truth is `crates/common/src/echelle.rs`.

## Status and Scope

- Status: active internal schema for MVP echelle extraction work
- Current supported major version: `1`
- Serialization formats:
  - TOML (`.toml`)
  - JSON (`.json`)
- Primary consumers:
  - UI `ImageViewerPanel` runtime profile cache and hot reload
  - Future extraction pipeline (local MVP and/or sidecar-backed)

## Design Goals

1. Encode all extraction-critical calibration information in a single versioned file.
2. Make runtime compatibility checks explicit (frame size, ROI, binning, bit depth).
3. Support both quick and high-fidelity wavelength representations:
   - polynomial fits
   - sampled wavelength arrays
4. Preserve provenance and calibration artifact references (blaze/flat/masks).
5. Provide strict validation with actionable error messages for operators.
6. Allow backward-compatible evolution within schema major version `1`.

## File Format and Loading Rules

## Extensions

- `.toml` and `.json` are supported.
- Any other extension is rejected before parsing.

## Load/Save Behavior

- `EchelleCalibrationProfile::load_from_path(...)`
  - detects format by file extension
  - parses profile
  - validates profile before returning it
- `EchelleCalibrationProfile::save_to_path(...)`
  - validates before serialization
  - writes TOML pretty-print or JSON pretty-print

## Version Compatibility Policy

## Schema Version Structure

`schema_version` is a semantic version triplet:

- `major`
- `minor`
- `patch`

## Read Compatibility (current implementation)

- rust-daq reads any profile with:
  - `schema_version.major == 1`
- rust-daq rejects profiles with:
  - `schema_version.major != 1`

This allows older `1.x.y` profiles to be loaded while preserving the original
minor/patch values in memory.

## Write Compatibility (current implementation)

- New profiles default to `1.0.0`.
- Writers should preserve loaded version metadata unless a migration step
  explicitly upgrades the schema representation.

## Top-Level Schema (v1)

`EchelleCalibrationProfile` contains:

- `schema_version` (`EchelleSchemaVersion`, default `1.0.0`)
- `profile_id` (`Option<String>`)
- `display_name` (`String`)
- `compatibility` (`EchelleFrameCompatibility`)
- `orientation` (`EchelleOrientation`)
- `extraction` (`EchelleExtractionConfig`)
- `orders` (`Vec<EchelleOrderCalibration>`)
- `corrections` (`EchelleCorrections`, default empty)
- `provenance` (`EchelleProvenance`)

## Field Details

## `schema_version`

- Purpose: declarative compatibility contract for parser/validator behavior.
- Validation:
  - `major` must match the supported major version (`1`)

## `profile_id`

- Optional stable identifier for operator workflows, caching, and audit trails.
- Suggested practice:
  - use a deterministic ID (instrument + camera mode + ROI + date)

## `display_name`

- Human-readable label shown in UI.
- Validation:
  - must not be empty / whitespace-only

## `compatibility` (`EchelleFrameCompatibility`)

Declares the detector geometry and acquisition mode assumptions the calibration
was built against.

Fields:

- `sensor_width`, `sensor_height`
  - full detector dimensions in pixels (unbinned sensor coordinates)
- `frame_width`, `frame_height`
  - dimensions of the delivered image frame used for extraction
- `roi_x`, `roi_y` (default `0`)
  - origin of the acquired frame in sensor coordinates
- `binning_x`, `binning_y` (default `1`)
  - detector binning factors for the frame
- `bit_depth` (`Option<u32>`)
  - optional runtime guardrail (e.g. 16-bit vs 12-bit packed modes)

Validation:

- sensor dims must be `> 0`
- frame dims must be `> 0`
- binning values must be `>= 1`
- `roi + frame` must fit within sensor bounds

Runtime frame compatibility checks (`validate_for_frame(...)`):

- frame width/height must match
- ROI X/Y must match when provided at runtime
- binning X/Y must match when provided at runtime
- bit depth must match when both expected and runtime values are present

Operational note:

- This is the main protection against accidentally applying a calibration from
  one camera mode/ROI to another.

## `orientation` (`EchelleOrientation`)

Defines how to interpret detector axes and order direction.

Fields:

- `dispersion_axis` (`x` or `y`)
- `cross_dispersion_axis` (`x` or `y`)
- `order_number_increase_direction` (`positive` or `negative`)
- `wavelength_increase_with_dispersion_positive` (`bool`)

Validation:

- `dispersion_axis` and `cross_dispersion_axis` must differ

Why this matters:

- The same extraction code can support rotated/flipped echellegrams when the
  profile encodes axis semantics rather than assuming a fixed orientation.

## `extraction` (`EchelleExtractionConfig`)

Default extraction policy used for runtime processing.

Fields:

- `summation_mode` (`EchelleSummationMode`)
  - `order_center_pixel`
  - `simple_sum`
  - `sqrt_weighted_sum`
  - `optimal`
- `default_aperture_half_width_px` (`f64`)
- `background` (`Option<EchelleBackgroundConfig>`)

Validation:

- `default_aperture_half_width_px` must be finite and `> 0`

### `extraction.background` (`EchelleBackgroundConfig`)

Fields:

- `enabled` (`bool`, default `false`)
- `inter_order_gap_min_px` (`u32`, default > 0)
- `baseline_window_px` (`u32`, default > 0)

Validation:

- `inter_order_gap_min_px > 0`
- `baseline_window_px > 0`

MVP note:

- Background configuration is stored now even if initial extraction stages use
  simpler subtraction or disable background handling.

## `orders` (`Vec<EchelleOrderCalibration>`)

Per-order calibration records. This is the core of the profile.

Each order contains:

- `relative_index` (`u32`)
  - zero-based order index within the profile
- `physical_order_number` (`Option<i32>`)
  - spectrograph/echelle physical order ID (m value for echelle equation)
  - automatically computed by the 3-pass pipeline using a robust cross-correlation search over candidate physical orders ($m \in [first\_m - 50, first\_m + 150]$) to handle sparse trace illumination safely (bd-ft49).
  - used by physics bootstrap (Pass 3) to predict wavelengths for uncalibrated orders
- `sample_start`, `sample_end` (`u32`, inclusive)
  - valid sample range along the dispersion axis for this order
- `trace` (`EchelleTraceModel`)
  - maps dispersion coordinate to order-center cross-dispersion position
- `wavelength` (`EchelleWavelengthModel`)
  - maps sample coordinate to wavelength
  - for arc-matched orders: fitted directly from atlas line matches
  - for bootstrapped orders: computed from 2D Chebyshev residual surface (Pass 3)
- `aperture_half_width_px` (`Option<f64>`)
  - per-order override of extraction aperture size
- `enabled` (`bool`, default `true`)
- `notes` (`Option<String>`)
  - may include flags like "arc-matched" or "bootstrapped" to indicate calibration source
  - populated by 3-pass pipeline with metadata about fit quality and method

Validation:

- `orders` must not be empty
- `relative_index` values must be unique
- `physical_order_number` values must be unique when present
- `sample_start <= sample_end`
- `sample_end` must not exceed the dispersion-axis frame length
- `aperture_half_width_px` (if present) must be finite and `> 0`

### Order Trace Representation (`EchelleTraceModel`)

v1 supports:

- `polynomial`

Fields:

- `basis` (`monomial` or `chebyshev`)
- `coefficients` (`Vec<f64>`)
- `domain_start`, `domain_end` (`f64`)

Validation:

- coefficients must be non-empty
- all coefficients must be finite
- domain must be finite and strictly increasing
- domain must cover `[sample_start, sample_end]`

Semantics:

- Input coordinate domain is expressed in detector sample coordinates along the
  dispersion axis.
- Output is the centerline location along the cross-dispersion axis.

### Wavelength Representation (`EchelleWavelengthModel`)

v1 supports two forms:

1. `polynomial`
2. `sampled`

#### `wavelength = { type = "polynomial", ... }`

Fields:

- `basis` (`monomial` or `chebyshev`)
- `coefficients` (`Vec<f64>`)
- `domain_start`, `domain_end` (`f64`)
- `unit` (`String`, required; e.g. `nm`)

Validation:

- `unit` must be non-empty
- coefficients must be non-empty and finite
- domain must be finite and strictly increasing
- domain must cover `[sample_start, sample_end]`

#### `wavelength = { type = "sampled", ... }`

Fields:

- `wavelengths` (`Vec<f64>`)
- `unit` (`String`, required)

Validation:

- `unit` must be non-empty
- `wavelengths.len()` must equal `(sample_end - sample_start + 1)`
- all wavelength samples must be finite
- wavelength samples must be strictly monotonic increasing

Why both representations exist:

- Polynomial is compact and convenient for fitted wavelength solutions.
- Sampled arrays preserve non-polynomial or empirically sampled solutions and
  avoid re-fitting artifacts when exporting from external tools.

## `corrections` (`EchelleCorrections`)

Optional artifact references and excluded regions.

Fields:

- `blaze` (`Option<EchelleArtifactRef>`)
- `flat_field` (`Option<EchelleArtifactRef>`)
- `bad_pixel_mask` (`Option<EchelleArtifactRef>`)
- `excluded_regions` (`Vec<PixelRegion>`)

### Artifact References (`EchelleArtifactRef`)

Fields:

- `path` (`String`)
- `sha256` (`Option<String>`)
- `format` (`Option<String>`)

Purpose:

- decouple large arrays / masks from the calibration profile while preserving
  a resolvable pointer plus integrity metadata

### Excluded Regions (`PixelRegion`)

Fields:

- `x`, `y`, `width`, `height`

Validation:

- `width > 0`
- `height > 0`

Use cases:

- hot columns / bad amplifier boundaries
- dust shadows
- persistent detector defects not represented in a full mask file

## `provenance` (`EchelleProvenance`)

Tracks origin and reproducibility context for the calibration.

Fields:

- `creator_tool` (`String`, required)
- `creator_version` (`Option<String>`)
- `created_at_utc` (`DateTime<Utc>`, required)
- `source_frame_ids` (`Vec<String>`)
- `notes` (`Option<String>`)

Validation:

- `creator_tool` must not be empty

Recommended practice:

- include IDs for flat, arc, and trace frames used to build calibration
- include notes for spectrograph config, slit, camera gain, and any manual edits

## Validation Error Philosophy

Validation errors are intentionally strict and operator-facing.

Goals:

- reject ambiguous/unsafe profiles early
- point to the field causing the issue
- make runtime mismatch failures actionable

Examples of actionable checks already implemented:

- ROI mismatch (`ROI X/Y mismatch`)
- binning mismatch (`binning_x` / `binning_y mismatch`)
- frame size mismatch
- unsupported file extension
- unsupported future major schema version

## Example (TOML, abridged)

The repository fixture is:

- `crates/common/tests/fixtures/echelle_profile_v1.toml`

Key structure:

```toml
schema_version = { major = 1, minor = 0, patch = 0 }
display_name = "Mechelle Demo Calibration (Fixture)"

[compatibility]
sensor_width = 2048
sensor_height = 2048
frame_width = 1024
frame_height = 512
roi_x = 128
roi_y = 256
binning_x = 1
binning_y = 1

[orientation]
dispersion_axis = "x"
cross_dispersion_axis = "y"

[extraction]
summation_mode = "simple_sum"
default_aperture_half_width_px = 4.0

[[orders]]
relative_index = 0
sample_start = 0
sample_end = 15
trace = { type = "polynomial", basis = "monomial", coefficients = [250.0, 0.0], domain_start = 0.0, domain_end = 1023.0 }
wavelength = { type = "sampled", wavelengths = [400.0, 400.02], unit = "nm" } # abridged in this example
```

## Migration Strategy (Future Schema Versions)

This section defines the planned migration policy for `bd-2kla.2.6`.

## Compatibility Contract

1. Major version changes (`2.x.y`) may break compatibility.
2. Minor version changes (`1.x.y`) must remain backward-readable by newer `1.*`
   readers whenever feasible.
3. Patch version changes (`1.0.x`) must be backward-readable and should reflect
   clarifications, bug fixes, or additive metadata only.

## Migration Mechanism (planned implementation pattern)

When a new schema version is introduced:

1. Parse into the corresponding versioned struct if format is recognized.
2. Normalize/migrate into the current in-memory canonical representation.
3. Preserve original source version metadata for audit/debugging where practical.
4. Re-validate after migration.

For now (v1-only), the code path is simpler:

- parse v1 profile
- validate v1

## Allowed Minor/Patch Additions in v1

Safe additive changes inside major `1` should prefer:

- new optional fields with defaults
- new enum variants only if all current readers can safely reject with clear
  messages (otherwise defer to next major)
- new provenance metadata
- new correction artifact references as optional fields

Avoid in-place semantic redefinition of existing fields in `1.x`.

## Backward Compatibility Test Requirements

Each schema evolution PR should include tests covering:

1. Loading the current fixture (`1.0.0`) still succeeds.
2. Loading an older `1.x.y` profile succeeds and preserves version values.
3. Loading a future major version fails with a clear error.
4. Runtime compatibility validation still reports ROI/binning mismatches with
   actionable messages.
5. Round-trip save/load for supported formats preserves semantically relevant
   fields.

Current tests covering this policy live in:

- `crates/common/src/echelle.rs`
- `crates/common/tests/echelle_profile_fixture.rs`

### Runtime UI Loading and Hot Reload Notes

The UI cache implementation is in:

- `crates/ui/src/panels/image_viewer/echelle_profile_cache.rs`

Behavior:

- caches the last-good parsed profile
- reloads when file modification time changes
- reports load/parse/validation errors
- preserves the previous valid profile if a reload fails

#### Merged Visualization logic

The "Merged" 1D spectrum viewer (`ImageViewerPanel`) combines multiple echelle orders into a single continuous plot.
- **Blaze Weighting**: Overlap regions are weighted by a theoretical sinc² blaze envelope. Weights are clamped to a `BLAZE_FLOOR` (0.1) to prevent noise amplification at detector edges (bd-rjhk).
- **Gap Handling**: To prevent drawing misleading interpolation lines between widely separated orders, the merging algorithm injects `f64::NAN` markers into the flux array whenever the wavelength gap between consecutive samples exceeds 2.0 nm (bd-rjhk).


This is intentional to support iterative calibration editing without breaking
live image viewing when a profile file is temporarily invalid during edits.
