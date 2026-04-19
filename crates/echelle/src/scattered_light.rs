//! Scattered light subtraction for echelle spectrographs.
//!
//! Two methods, selected via `ScatteredLightConfig::method`:
//!
//! - **`InterOrderMedian`** (default, CERES-style): build an inter-order
//!   mask from the detected trace positions, bin the frame into blocks,
//!   take a sigma-clipped median of inter-order pixels per block, fit a
//!   bivariate Chebyshev surface to the block medians.
//! - **`MorphologicalOpening`** (bd-vdfum / NotebookLM §FM4 for ICCD):
//!   apply a vertical morphological opening (erosion → dilation with a
//!   25-px vertical structuring element) so order-width + MCP-halo peaks
//!   are obliterated and only the slowly-varying background survives;
//!   sample a sparse grid of anchors from the opened image, excluding
//!   any anchor within a 15-px radius of a saturated pixel; fit the same
//!   2D Chebyshev surface to those anchors. Required on MCP-intensified
//!   detectors where the halo floods the inter-order gap and corrupts
//!   the standard CERES median (documented "4500-count baseline
//!   anomaly" on the Mechelle 5000 + iStar).
//!
//! Source: NotebookLM echelle notebook 7f275c3a, pipeline evaluation
//! memo 3a0a13df §FM4 "The 4500-Count Baseline Anomaly".

// Pixel-index casts: always small enough for lossless usize→f64 conversion,
// and f64→usize truncation is intentional for pixel coordinates.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::types::EchelleTraceModel;

/// Which scattered-light estimation method to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScatteredLightMethod {
    /// CERES-style: sigma-clipped median of inter-order pixels per block.
    /// The default for conventional CCDs and DH3P-flat-dominated frames.
    InterOrderMedian,
    /// Morphological opening (25-px vertical structuring element) + sparse
    /// anchor sampling on the opened image, excluding a 15-px radius
    /// around saturated pixels. Required for MCP-intensified detectors
    /// (bd-vdfum / NotebookLM §FM4) where the halo floods inter-order
    /// gaps and corrupts the standard median. See module docs.
    MorphologicalOpening,
}

/// Configuration for scattered light subtraction.
///
/// Presence or absence of scatter subtraction is controlled at the CALL SITE
/// rather than by a field on this struct: `CalibrationPipelineConfig` carries
/// separate `arc_scatter: Option<ScatteredLightConfig>` and
/// `flat_scatter: Option<ScatteredLightConfig>` fields. A `None` means "skip
/// this stage for that frame type" (bd-g22gu.3). This replaced the previous
/// single `scatter_config: Option<_>` + redundant `enabled: bool` field.
#[derive(Debug, Clone)]
pub struct ScatteredLightConfig {
    /// Estimation method. Default is `InterOrderMedian` (conventional CCD);
    /// set to `MorphologicalOpening` for MCP-intensified detectors
    /// (iStar ICCD) per NotebookLM §FM4.
    pub method: ScatteredLightMethod,
    /// Aperture half-width used to define order regions (pixels). Only
    /// used by `InterOrderMedian`.
    pub aperture_half_width: f64,
    /// Block size / anchor grid step for spatial binning (pixels, default:
    /// 64). For `MorphologicalOpening` this is the grid stride.
    pub block_size: u32,
    /// Polynomial degree in the dispersion direction (default: 3).
    pub poly_degree_x: usize,
    /// Polynomial degree in the cross-dispersion direction (default: 3).
    pub poly_degree_y: usize,
    /// For `MorphologicalOpening` only: vertical structuring-element size
    /// in pixels. NotebookLM §FM4 recommends 25 px for Mechelle 5000
    /// (≈ 5 px order width + 8 px halo on each side).
    pub morph_vertical_kernel_px: u32,
    /// For `MorphologicalOpening` only: exclusion radius (pixels) around
    /// saturated pixels when sampling anchors. Default 15 per NotebookLM.
    pub morph_saturation_exclusion_px: u32,
    /// For `MorphologicalOpening` only: threshold (as a fraction of the
    /// frame's max ADU) above which a pixel is considered "saturated"
    /// for anchor exclusion. Default 0.95.
    pub morph_saturation_frac: f64,
}

impl Default for ScatteredLightConfig {
    fn default() -> Self {
        Self {
            method: ScatteredLightMethod::InterOrderMedian,
            aperture_half_width: 5.0,
            block_size: 64,
            poly_degree_x: 3,
            poly_degree_y: 3,
            morph_vertical_kernel_px: 25,
            morph_saturation_exclusion_px: 15,
            morph_saturation_frac: 0.95,
        }
    }
}

impl ScatteredLightConfig {
    /// Preset for DH3P / continuum flats captured on the Mechelle 5000 +
    /// iStar ICCD (bd-g22gu.3): morphological opening with the NotebookLM
    /// §FM4 parameters tuned for the NIR-order 9-px inter-order gap.
    ///
    /// This is the **default for `CalibrationPipelineConfig::flat_scatter`**
    /// on ME5000 configs. The opened-surface subtraction removes the
    /// 4500-count MCP halo baseline from the flat before trace detection
    /// and blaze extraction, recovering inter-element intensity ratios for
    /// downstream LIBS science (bd-3yb8).
    ///
    /// **Do NOT use this preset for arc frames** (HgAr, ThAr) — empirical
    /// validation on the leabs-dev HgAr capture shows that morphological
    /// opening over-subtracts on pure-emission-line sources: with kernel=25
    /// only 3/7 expected Hg lines survive the downstream atlas match; with
    /// kernel=13 only 1/7 does. The call-site split in
    /// `CalibrationPipelineConfig` (`arc_scatter` vs `flat_scatter`) means
    /// this preset can only land on the flat by construction.
    #[must_use]
    pub fn mechelle_5000_istar_flat() -> Self {
        Self {
            method: ScatteredLightMethod::MorphologicalOpening,
            aperture_half_width: 5.0,
            block_size: 64,
            poly_degree_x: 3,
            poly_degree_y: 3,
            morph_vertical_kernel_px: 13,
            morph_saturation_exclusion_px: 15,
            morph_saturation_frac: 0.95,
        }
    }
}

/// Result of scattered light estimation.
#[derive(Debug, Clone)]
pub struct ScatteredLightModel {
    /// Chebyshev coefficients for the 2D surface.
    /// Layout: `coeffs[iy * (degree_x+1) + ix]` for T_ix(x_norm) * T_iy(y_norm).
    pub coefficients: Vec<f64>,
    /// Polynomial degree in x (dispersion).
    pub degree_x: usize,
    /// Polynomial degree in y (cross-dispersion).
    pub degree_y: usize,
    /// Frame width used for normalization.
    pub width: u32,
    /// Frame height used for normalization.
    pub height: u32,
}

impl ScatteredLightModel {
    /// Evaluate the scattered light model at a pixel position.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        let x_norm = 2.0 * x / f64::from(self.width.max(1)) - 1.0;
        let y_norm = 2.0 * y / f64::from(self.height.max(1)) - 1.0;
        eval_2d_chebyshev(
            &self.coefficients,
            self.degree_x,
            self.degree_y,
            x_norm,
            y_norm,
        )
    }

    /// Subtract scattered light from a frame in-place.
    ///
    /// Fast path: precompute per-column `T_ix(x_norm)` and per-row `T_iy(y_norm)`
    /// basis tables once, then factor the 2D evaluation into an outer sum over
    /// `iy` (computed once per row) and an inner sum over `ix` (computed once
    /// per pixel). This replaces 2·W·H short-lived `Vec<f64>` allocations and
    /// a per-pixel `O(nx·ny)` inner loop with zero allocations and an
    /// `O(nx)` inner loop (bd-gn016).
    ///
    /// Bit-equivalent to `eval(col, row)` per pixel up to floating-point
    /// summation order (differences are sub-ULP and absorbed by downstream
    /// tolerances).
    pub fn subtract_from(&self, frame: &mut [f32], width: u32) {
        let w = width as usize;
        if w == 0 {
            return;
        }
        let h = frame.len() / w;
        let nx = self.degree_x + 1;
        let ny = self.degree_y + 1;
        let w_norm = f64::from(self.width.max(1));
        let h_norm = f64::from(self.height.max(1));

        // Per-column basis table: tx_table[col * nx + i] = T_i(x_norm(col)).
        // Sizing: w * nx * 8 bytes ≤ 2560 * 8 * 8 ≈ 160 KB for ME5000 + nx≤8.
        let mut tx_table = vec![0.0_f64; w * nx];
        for col in 0..w {
            let x_norm = 2.0 * (col as f64) / w_norm - 1.0;
            crate::chebyshev_common::chebyshev_basis_into(
                x_norm,
                &mut tx_table[col * nx..(col + 1) * nx],
            );
        }

        // Per-row scratch: ty[iy] = T_iy(y_norm) for the current row.
        // row_contrib[ix] = sum_iy c[iy*nx + ix] * ty[iy] — folds out y so the
        // inner per-pixel loop is just a dot product of tx·row_contrib.
        let mut ty = vec![0.0_f64; ny];
        let mut row_contrib = vec![0.0_f64; nx];

        for row in 0..h {
            let y_norm = 2.0 * (row as f64) / h_norm - 1.0;
            crate::chebyshev_common::chebyshev_basis_into(y_norm, &mut ty);

            // Layout: coeffs[iy * nx + ix] (y-outer, x-inner).
            for (ix, contrib) in row_contrib.iter_mut().enumerate() {
                let mut s = 0.0_f64;
                for (iy, &ty_val) in ty.iter().enumerate() {
                    s += self.coefficients[iy * nx + ix] * ty_val;
                }
                *contrib = s;
            }

            let row_base = row * w;
            for col in 0..w {
                let tx = &tx_table[col * nx..(col + 1) * nx];
                let mut scatter = 0.0_f64;
                for (tx_val, contrib) in tx.iter().zip(row_contrib.iter()) {
                    scatter += tx_val * contrib;
                }
                let pixel = &mut frame[row_base + col];
                *pixel = (f64::from(*pixel) - scatter).max(0.0) as f32;
            }
        }
    }
}

/// Trace specification for building the inter-order mask.
pub struct TraceInfo<'a> {
    /// Trace model for this order.
    pub trace: &'a EchelleTraceModel,
    /// Starting dispersion pixel.
    pub disp_start: u32,
    /// Ending dispersion pixel.
    pub disp_end: u32,
}

/// Estimate and subtract scattered light from a frame.
///
/// Returns a new frame with scattered light subtracted, plus the model.
pub fn subtract_scattered_light(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    config: &ScatteredLightConfig,
) -> Option<(Vec<f32>, ScatteredLightModel)> {
    let model = estimate_scattered_light(frame, width, height, traces, config)?;
    let mut corrected = frame.to_vec();
    model.subtract_from(&mut corrected, width);
    Some((corrected, model))
}

/// Estimate the scattered light model without subtracting.
pub fn estimate_scattered_light(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    config: &ScatteredLightConfig,
) -> Option<ScatteredLightModel> {
    let w = width as usize;
    let h = height as usize;
    if frame.len() < w * h || w < 2 || h < 2 {
        return None;
    }

    match config.method {
        ScatteredLightMethod::InterOrderMedian => {
            estimate_inter_order_median(frame, width, height, traces, config)
        }
        ScatteredLightMethod::MorphologicalOpening => {
            estimate_morphological_opening(frame, width, height, config)
        }
    }
}

/// CERES-style estimation: sigma-clipped median of inter-order pixels.
fn estimate_inter_order_median(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    config: &ScatteredLightConfig,
) -> Option<ScatteredLightModel> {
    let w = width as usize;
    let h = height as usize;
    // Step 1: Build inter-order mask.
    let inter_order_mask =
        build_inter_order_mask(width, height, traces, config.aperture_half_width);

    // Step 2: Bin into blocks and compute median of inter-order pixels.
    let bs = config.block_size.max(1) as usize;
    let n_blocks_x = w.div_ceil(bs);
    let n_blocks_y = h.div_ceil(bs);

    let mut block_x = Vec::new();
    let mut block_y = Vec::new();
    let mut block_val = Vec::new();

    let mut scratch = Vec::new();

    for by in 0..n_blocks_y {
        for bx in 0..n_blocks_x {
            scratch.clear();
            let y_start = by * bs;
            let y_end = ((by + 1) * bs).min(h);
            let x_start = bx * bs;
            let x_end = ((bx + 1) * bs).min(w);

            for row in y_start..y_end {
                for col in x_start..x_end {
                    let idx = row * w + col;
                    if inter_order_mask[idx] {
                        scratch.push(f64::from(frame[idx]));
                    }
                }
            }

            if scratch.len() >= 3 {
                // Sigma-clipped median (bd-8yjd1 P2.5): a single cosmic-ray
                // hit inside an inter-order block used to corrupt the block
                // median, which then distorted the 2D Chebyshev background
                // fit. Clip values > 5σ from the current median for up to 3
                // passes before taking the final median.
                let robust = sigma_clipped_median(&mut scratch, 5.0, 3);
                block_x.push(x_start.midpoint(x_end) as f64);
                block_y.push(y_start.midpoint(y_end) as f64);
                block_val.push(robust);
            }
        }
    }

    let n_coeffs = (config.poly_degree_x + 1) * (config.poly_degree_y + 1);
    if block_val.len() < n_coeffs + 1 {
        return None;
    }

    // Step 3: Fit 2D Chebyshev surface.
    let x_norm: Vec<f64> = block_x
        .iter()
        .map(|&x| 2.0 * x / f64::from(width) - 1.0)
        .collect();
    let y_norm: Vec<f64> = block_y
        .iter()
        .map(|&y| 2.0 * y / f64::from(height) - 1.0)
        .collect();

    let coefficients = fit_2d_chebyshev(
        &x_norm,
        &y_norm,
        &block_val,
        config.poly_degree_x,
        config.poly_degree_y,
    )?;

    Some(ScatteredLightModel {
        coefficients,
        degree_x: config.poly_degree_x,
        degree_y: config.poly_degree_y,
        width,
        height,
    })
}

/// bd-vdfum / NotebookLM §FM4: morphological opening + sparse anchor fit.
///
/// 1. Apply a vertical morphological opening with a
///    `config.morph_vertical_kernel_px`-wide structuring element. Erosion
///    replaces each pixel with the min over a ±k/2 vertical neighborhood
///    (squashing spectral lines + their MCP halos toward the true
///    background). Dilation then expands back (max over the same
///    neighborhood), restoring the spatial scale without re-introducing
///    the peaks.
/// 2. Detect saturated pixels (≥ `morph_saturation_frac × frame_max`).
///    Grow a `morph_saturation_exclusion_px`-radius exclusion mask
///    around each saturated pixel.
/// 3. Sample the opened image on a regular grid (`block_size` stride),
///    skipping grid points inside the exclusion mask.
/// 4. Fit the existing 2D Chebyshev surface to the samples. The low
///    Chebyshev degree (default 3×3) provides the "exceptionally high
///    stiffness" NotebookLM §FM4 prescribes, without needing TPS/RBF.
fn estimate_morphological_opening(
    frame: &[f32],
    width: u32,
    height: u32,
    config: &ScatteredLightConfig,
) -> Option<ScatteredLightModel> {
    let w = width as usize;
    let h = height as usize;
    let kernel = config.morph_vertical_kernel_px.max(3) as usize;
    let excl_r = config.morph_saturation_exclusion_px.max(1) as usize;

    // Step 1: vertical erosion then dilation (opening).
    let eroded = vertical_minimum_filter(frame, w, h, kernel);
    let opened = vertical_maximum_filter(&eroded, w, h, kernel);

    // Step 2: saturation exclusion mask.
    let frame_max = frame
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0.0f32, f32::max);
    let sat_threshold = frame_max * config.morph_saturation_frac as f32;
    let excl_mask = build_saturation_exclusion_mask(frame, w, h, sat_threshold, excl_r);

    // Step 3: anchor grid on the opened image, skipping excluded positions.
    let stride = config.block_size.max(4) as usize;
    let mut anchor_x = Vec::new();
    let mut anchor_y = Vec::new();
    let mut anchor_val = Vec::new();
    for row in (stride / 2..h).step_by(stride) {
        for col in (stride / 2..w).step_by(stride) {
            let idx = row * w + col;
            if excl_mask[idx] {
                continue;
            }
            let v = opened[idx];
            if !v.is_finite() {
                continue;
            }
            anchor_x.push(col as f64);
            anchor_y.push(row as f64);
            anchor_val.push(f64::from(v));
        }
    }

    let n_coeffs = (config.poly_degree_x + 1) * (config.poly_degree_y + 1);
    if anchor_val.len() < n_coeffs + 1 {
        return None;
    }

    tracing::info!(
        n_anchors = anchor_val.len(),
        frame_max = frame_max,
        sat_threshold = sat_threshold,
        "scattered_light[morph]: sampled anchors from opened image"
    );

    // Step 4: fit 2D Chebyshev surface (reuse existing fitter).
    let x_norm: Vec<f64> = anchor_x
        .iter()
        .map(|&x| 2.0 * x / f64::from(width) - 1.0)
        .collect();
    let y_norm: Vec<f64> = anchor_y
        .iter()
        .map(|&y| 2.0 * y / f64::from(height) - 1.0)
        .collect();

    let coefficients = fit_2d_chebyshev(
        &x_norm,
        &y_norm,
        &anchor_val,
        config.poly_degree_x,
        config.poly_degree_y,
    )?;

    Some(ScatteredLightModel {
        coefficients,
        degree_x: config.poly_degree_x,
        degree_y: config.poly_degree_y,
        width,
        height,
    })
}

/// Compile-time selector for the vertical extremum filter direction
/// (bd-xhqdo). Pre-bd-xhqdo the crate shipped two ~95%-duplicated deque
/// implementations of the rolling-window min/max along the cross-dispersion
/// axis; unifying them with a zero-sized trait parameter collapses both
/// paths into a single body that monomorphizes to the exact old code.
trait VerticalExtremum {
    /// Return `true` when the back element of the deque is dominated by the
    /// new candidate and should be dropped (strict-better plus non-finite).
    /// For a min filter this is `back >= new`; for a max filter `back <= new`.
    fn should_pop_back(back: f32, new: f32) -> bool;

    /// Neutral initial value used by the tail-flush naive scan when the
    /// window straddles the far edge of the column. `+∞` for min, `-∞` for max.
    const FLUSH_INIT: f32;

    /// True when `candidate` improves on `current_best` in the flush scan.
    fn is_better(candidate: f32, current_best: f32) -> bool;
}

struct Min;
impl VerticalExtremum for Min {
    #[inline]
    fn should_pop_back(back: f32, new: f32) -> bool {
        !back.is_finite() || back >= new
    }
    const FLUSH_INIT: f32 = f32::INFINITY;
    #[inline]
    fn is_better(candidate: f32, current_best: f32) -> bool {
        candidate < current_best
    }
}

struct Max;
impl VerticalExtremum for Max {
    #[inline]
    fn should_pop_back(back: f32, new: f32) -> bool {
        !back.is_finite() || back <= new
    }
    const FLUSH_INIT: f32 = f32::NEG_INFINITY;
    #[inline]
    fn is_better(candidate: f32, current_best: f32) -> bool {
        candidate > current_best
    }
}

/// Deque-based O(w·h) rolling-window extremum filter along the cross-dispersion
/// (y / row) axis. Each output pixel is the min or max — as determined by the
/// `E` type parameter — over a ±(kernel/2) vertical neighborhood of the input.
///
/// Instantiated as `::<Min>` for erosion and `::<Max>` for dilation; both
/// monomorphize to byte-equivalent code with the pre-bd-xhqdo implementations.
fn vertical_extremum_filter<E: VerticalExtremum>(
    frame: &[f32],
    w: usize,
    h: usize,
    kernel: usize,
) -> Vec<f32> {
    let half = kernel / 2;
    let mut out = vec![f32::NAN; w * h];
    let mut col_buf = vec![0.0f32; h];
    let mut deque: std::collections::VecDeque<usize> = std::collections::VecDeque::with_capacity(h);
    for col in 0..w {
        for row in 0..h {
            col_buf[row] = frame[row * w + col];
        }
        deque.clear();
        for row in 0..h {
            // Drop from tail while the new value dominates the tail (min:
            // ≥, max: ≤, plus non-finite). Monomorphized: const-folded to
            // the concrete comparator per instantiation.
            while let Some(&back) = deque.back() {
                if E::should_pop_back(col_buf[back], col_buf[row]) {
                    deque.pop_back();
                } else {
                    break;
                }
            }
            deque.push_back(row);
            // Drop from head rows that are now outside the symmetric ±half
            // window centered at `center = row - half`. The valid window is
            // [center - half, center + half] = [row - 2·half, row], so drop
            // rows with index < row - 2·half. Pre-bd-xhqdo the code used
            // `row - half` here, which effectively emitted the min/max over
            // the asymmetric upper half [center, center + half] only — a
            // pre-existing bug filed and fixed in this PR (bd-g22gu.1.2).
            let lo = row.saturating_sub(2 * half);
            while let Some(&front) = deque.front() {
                if front < lo {
                    deque.pop_front();
                } else {
                    break;
                }
            }
            // Emit once the window has slid past the upper edge so that
            // (row - half) is the center pixel of the kernel.
            if row >= half {
                let center = row - half;
                out[center * w + col] = col_buf[*deque.front().unwrap()];
            }
        }
        // Flush the trailing rows (where center + half ≥ h) with a naive
        // scan of the truncated window.
        for center in (h.saturating_sub(half))..h {
            let lo = center.saturating_sub(half);
            let hi = (center + half).min(h - 1);
            let mut m = E::FLUSH_INIT;
            for &v in &col_buf[lo..=hi] {
                if v.is_finite() && E::is_better(v, m) {
                    m = v;
                }
            }
            out[center * w + col] = if m.is_finite() { m } else { 0.0 };
        }
    }
    out
}

/// Vertical minimum filter (grayscale erosion with a vertical line kernel).
/// Each output pixel is the min over a ±(k/2) vertical neighborhood of the
/// input. Uses a rolling window deque for O(w·h) total cost regardless of
/// kernel size. See [`vertical_extremum_filter`].
fn vertical_minimum_filter(frame: &[f32], w: usize, h: usize, kernel: usize) -> Vec<f32> {
    vertical_extremum_filter::<Min>(frame, w, h, kernel)
}

/// Vertical maximum filter (grayscale dilation with a vertical line kernel).
/// See [`vertical_extremum_filter`].
fn vertical_maximum_filter(frame: &[f32], w: usize, h: usize, kernel: usize) -> Vec<f32> {
    vertical_extremum_filter::<Max>(frame, w, h, kernel)
}

/// Grow a boolean mask where `true` means "within `radius` pixels of a
/// saturated pixel". Saturated pixels are `frame[i] >= threshold`.
fn build_saturation_exclusion_mask(
    frame: &[f32],
    w: usize,
    h: usize,
    threshold: f32,
    radius: usize,
) -> Vec<bool> {
    let mut saturated = vec![false; w * h];
    for (i, &v) in frame.iter().enumerate() {
        if v.is_finite() && v >= threshold {
            saturated[i] = true;
        }
    }
    // Dilate the saturation mask by `radius` in both axes via a 2-pass
    // vertical-then-horizontal or-filter (Minkowski sum with a square
    // kernel). For 15-px radius this is cheap.
    let mut out = saturated.clone();
    // Horizontal pass.
    for row in 0..h {
        let row_base = row * w;
        // Rolling window: count saturated pixels in [col-radius, col+radius].
        let mut count = 0usize;
        for c in 0..radius.min(w) {
            if saturated[row_base + c] {
                count += 1;
            }
        }
        for col in 0..w {
            // Add the column that just entered the window (col + radius).
            if col + radius < w && saturated[row_base + col + radius] {
                count += 1;
            }
            // Remove the column that just left (col - radius - 1).
            if col > radius && saturated[row_base + col - radius - 1] {
                count -= 1;
            }
            if count > 0 {
                out[row_base + col] = true;
            }
        }
    }
    let horiz = out.clone();
    // Vertical pass on the horizontally-dilated mask.
    for col in 0..w {
        let mut count = 0usize;
        for r in 0..radius.min(h) {
            if horiz[r * w + col] {
                count += 1;
            }
        }
        for row in 0..h {
            if row + radius < h && horiz[(row + radius) * w + col] {
                count += 1;
            }
            if row > radius && horiz[(row - radius - 1) * w + col] {
                count -= 1;
            }
            if count > 0 {
                out[row * w + col] = true;
            }
        }
    }
    out
}

/// Build a mask where `true` means "inter-order pixel" (not covered by any trace).
fn build_inter_order_mask(
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    aperture_half_width: f64,
) -> Vec<bool> {
    let w = width as usize;
    let h = height as usize;
    let mut mask = vec![true; w * h];

    for trace_info in traces {
        for col in trace_info.disp_start..=trace_info.disp_end.min(width - 1) {
            if let Some(center) = eval_trace_safe(trace_info.trace, f64::from(col)) {
                let y_min = (center - aperture_half_width).floor().max(0.0) as usize;
                let y_max = (center + aperture_half_width).ceil().min(h as f64 - 1.0) as usize;
                for row in y_min..=y_max {
                    mask[row * w + col as usize] = false;
                }
            }
        }
    }

    mask
}

/// Evaluate a 2D Chebyshev polynomial: sum_{i,j} c_{ij} * T_i(x) * T_j(y).
///
/// Layout: `coeffs[iy * nx + ix]` — y is the outer (slower-varying) index,
/// x is the inner (faster-varying) index. This is the opposite layout from
/// `wavelength_fitting::chebyshev_eval_2d` / `chebyshev_2d::eval_chebyshev_2d_surface`,
/// which both store x outer. The shared helper handles either by taking the
/// outer/inner dimensions in call order.
fn eval_2d_chebyshev(
    coeffs: &[f64],
    degree_x: usize,
    degree_y: usize,
    x_norm: f64,
    y_norm: f64,
) -> f64 {
    crate::chebyshev_common::eval_2d(coeffs, degree_y + 1, degree_x + 1, y_norm, x_norm)
}

/// Fit 2D Chebyshev coefficients via least-squares normal equations.
fn fit_2d_chebyshev(
    x_norm: &[f64],
    y_norm: &[f64],
    values: &[f64],
    degree_x: usize,
    degree_y: usize,
) -> Option<Vec<f64>> {
    let n_pts = x_norm.len();
    let nx = degree_x + 1;
    let ny = degree_y + 1;
    let n_coeffs = nx * ny;

    if n_pts < n_coeffs {
        return None;
    }

    // Flat row-major Vandermonde: V[row, iy*nx + ix] = T_ix(x_row) * T_iy(y_row).
    let mut vander = vec![0.0_f64; n_pts * n_coeffs];
    for (row, (&xn, &yn)) in x_norm.iter().zip(y_norm).enumerate() {
        let tx = crate::chebyshev_common::chebyshev_basis(xn, nx);
        let ty = crate::chebyshev_common::chebyshev_basis(yn, ny);
        let row_base = row * n_coeffs;
        for iy in 0..ny {
            for ix in 0..nx {
                vander[row_base + iy * nx + ix] = tx[ix] * ty[iy];
            }
        }
    }

    // V^T V c = V^T y via the shared helper (bd-g22gu.1.1).
    crate::chebyshev_common::solve_least_squares_flat(&vander, n_pts, n_coeffs, values)
}

/// Safely evaluate a trace model, returning None on errors.
fn eval_trace_safe(trace: &EchelleTraceModel, x: f64) -> Option<f64> {
    match trace {
        EchelleTraceModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
        } => {
            if coefficients.is_empty() || !x.is_finite() || *domain_start >= *domain_end {
                return None;
            }
            let result = match basis {
                crate::types::PolynomialBasis::Monomial => {
                    let mut acc = 0.0f64;
                    for &c in coefficients.iter().rev() {
                        acc = acc * x + c;
                    }
                    acc
                }
                crate::types::PolynomialBasis::Chebyshev => {
                    let t = (2.0 * (x - domain_start)) / (domain_end - domain_start) - 1.0;
                    if coefficients.len() == 1 {
                        return Some(coefficients[0]);
                    }
                    let mut t0 = 1.0f64;
                    let mut t1 = t;
                    let mut acc = coefficients[0] * t0 + coefficients[1] * t1;
                    for &c in coefficients.iter().skip(2) {
                        let tn = 2.0 * t * t1 - t0;
                        acc += c * tn;
                        t0 = t1;
                        t1 = tn;
                    }
                    acc
                }
            };
            if result.is_finite() {
                Some(result)
            } else {
                None
            }
        }
    }
}

// ─── Fast path for live extraction ─────────────────────────────────────────

/// Estimate a full-resolution scattered-light map using the CERES coarse-grid method.
///
/// 1. On a coarse grid (`col_stride × row_stride`), sample the median of
///    inter-order pixel values.
/// 2. Bilinearly interpolate to full resolution.
///
/// Returns a `Vec<f32>` of size `width × height` containing the estimated
/// scatter value at each pixel. Use this with [`subtract_scatter_map`] or
/// pass it into the extraction pipeline for per-pixel correction.
pub fn estimate_scatter_map_fast_f32(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    aperture_half_width: f64,
    col_stride: u32,
    row_stride: u32,
) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    if frame.len() < w * h || w < 2 || h < 2 {
        return vec![0.0; w * h];
    }

    let cs = col_stride.max(1) as usize;
    let rs = row_stride.max(1) as usize;
    let grid_cols = w.div_ceil(cs);
    let grid_rows = h.div_ceil(rs);

    // Build coarse scattered-light map: for each grid cell, compute the
    // median of inter-order pixels in a small window around the grid point.
    let mut coarse_map = vec![0.0f32; grid_cols * grid_rows];
    let mut scratch = Vec::new();

    for gy in 0..grid_rows {
        let cy = (gy * rs + rs / 2).min(h - 1);
        let row_lo = cy.saturating_sub(rs / 2);
        let row_hi = (cy + rs / 2 + 1).min(h);

        for gx in 0..grid_cols {
            let cx = (gx * cs + cs / 2).min(w - 1);
            let col_lo = cx.saturating_sub(cs / 2);
            let col_hi = (cx + cs / 2 + 1).min(w);

            scratch.clear();
            for row in row_lo..row_hi {
                for col in col_lo..col_hi {
                    if is_inter_order(col as f64, row as f64, traces, aperture_half_width) {
                        let val = frame[row * w + col];
                        if val.is_finite() {
                            scratch.push(val);
                        }
                    }
                }
            }

            coarse_map[gy * grid_cols + gx] = if scratch.is_empty() {
                0.0
            } else {
                scratch
                    .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                scratch[scratch.len() / 2]
            };
        }
    }

    // Bilinear interpolation → full resolution scatter map.
    let mut scatter_map = vec![0.0f32; w * h];
    for row in 0..h {
        let gy_f = row as f64 / rs as f64;
        let gy0 = (gy_f.floor() as usize).min(grid_rows.saturating_sub(2));
        let gy1 = (gy0 + 1).min(grid_rows - 1);
        let ty = (gy_f - gy0 as f64).clamp(0.0, 1.0) as f32;

        for col in 0..w {
            let gx_f = col as f64 / cs as f64;
            let gx0 = (gx_f.floor() as usize).min(grid_cols.saturating_sub(2));
            let gx1 = (gx0 + 1).min(grid_cols - 1);
            let tx = (gx_f - gx0 as f64).clamp(0.0, 1.0) as f32;

            let v00 = coarse_map[gy0 * grid_cols + gx0];
            let v10 = coarse_map[gy0 * grid_cols + gx1];
            let v01 = coarse_map[gy1 * grid_cols + gx0];
            let v11 = coarse_map[gy1 * grid_cols + gx1];

            scatter_map[row * w + col] = v00 * (1.0 - tx) * (1.0 - ty)
                + v10 * tx * (1.0 - ty)
                + v01 * (1.0 - tx) * ty
                + v11 * tx * ty;
        }
    }
    scatter_map
}

/// Subtract a pre-computed scatter map from a frame in-place.
pub fn subtract_scatter_map(frame: &mut [f32], scatter_map: &[f32]) {
    for (pixel, &scatter) in frame.iter_mut().zip(scatter_map.iter()) {
        *pixel = (*pixel - scatter).max(0.0);
    }
}

/// Fast scattered light subtraction for live frame preview.
///
/// Convenience wrapper: estimates + subtracts in one call.
/// See [`estimate_scatter_map_fast_f32`] for the algorithm details.
pub fn subtract_scattered_light_fast_f32(
    frame: &mut [f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    aperture_half_width: f64,
    col_stride: u32,
    row_stride: u32,
) {
    let scatter_map = estimate_scatter_map_fast_f32(
        frame,
        width,
        height,
        traces,
        aperture_half_width,
        col_stride,
        row_stride,
    );
    subtract_scatter_map(frame, &scatter_map);
}

/// Check whether a pixel is inter-order (not within any trace aperture).
fn is_inter_order(col: f64, row: f64, traces: &[TraceInfo<'_>], aperture_half_width: f64) -> bool {
    for trace_info in traces {
        let col_u32 = col as u32;
        if col_u32 < trace_info.disp_start || col_u32 > trace_info.disp_end {
            continue;
        }
        if let Some(center) = eval_trace_safe(trace_info.trace, col)
            && (row - center).abs() <= aperture_half_width
        {
            return false;
        }
    }
    true
}

/// Median of `values` after `max_iters` passes of σ-clipping at `clip_sigma`.
///
/// Each pass computes the current median and MAD-based σ, drops values whose
/// |x − median| > `clip_sigma · σ`, and re-sorts. Returns early when a pass
/// removes nothing. If the input has fewer than 3 samples it falls back to
/// the plain median. `values` is sorted in place.
fn sigma_clipped_median(values: &mut Vec<f64>, clip_sigma: f64, max_iters: usize) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    for _ in 0..max_iters {
        let n = values.len();
        if n < 3 {
            break;
        }
        let median = values[n / 2];
        // MAD: median absolute deviation; robust σ ≈ 1.4826 · MAD.
        let mut abs_dev: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
        abs_dev.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mad = abs_dev[abs_dev.len() / 2];
        let sigma = (1.4826 * mad).max(1e-12);
        let threshold = clip_sigma * sigma;
        let before = values.len();
        values.retain(|&v| (v - median).abs() <= threshold);
        if values.len() == before || values.len() < 3 {
            break;
        }
    }
    values[values.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PolynomialBasis;

    fn flat_trace(center: f64) -> EchelleTraceModel {
        EchelleTraceModel::Polynomial {
            basis: PolynomialBasis::Monomial,
            coefficients: vec![center],
            domain_start: 0.0,
            domain_end: 1000.0,
        }
    }

    #[test]
    fn test_inter_order_mask() {
        let width = 100;
        let height = 50;
        let traces = [
            TraceInfo {
                trace: &flat_trace(15.0),
                disp_start: 0,
                disp_end: width - 1,
            },
            TraceInfo {
                trace: &flat_trace(35.0),
                disp_start: 0,
                disp_end: width - 1,
            },
        ];

        let mask = build_inter_order_mask(width, height, &traces, 3.0);

        // Pixels near trace centers should be masked out (false).
        assert!(
            !mask[15 * width as usize + 50],
            "pixel at trace center should be masked"
        );
        assert!(
            !mask[13 * width as usize + 50],
            "pixel within aperture should be masked"
        );

        // Pixels far from both traces should be unmasked (true).
        assert!(
            mask[50], // row 0, col 50
            "pixel far from traces should be inter-order"
        );
        assert!(
            mask[25 * width as usize + 50],
            "pixel between traces should be inter-order"
        );
    }

    #[test]
    fn test_constant_scattered_light() {
        // Frame with uniform background of 50 + no orders.
        let width: u32 = 128;
        let height: u32 = 64;
        let background = 50.0f32;
        let frame = vec![background; width as usize * height as usize];

        let config = ScatteredLightConfig {
            block_size: 32,
            poly_degree_x: 1,
            poly_degree_y: 1,
            aperture_half_width: 3.0,
            ..ScatteredLightConfig::default()
        };

        let (corrected, model) =
            subtract_scattered_light(&frame, width, height, &[], &config).expect("should fit");

        // After subtraction, all pixels should be near zero.
        for (i, &val) in corrected.iter().enumerate() {
            assert!(val.abs() < 1.0, "pixel {i}: corrected={val}, expected ~0");
        }

        // Model should evaluate to ~50 everywhere.
        let center_val = model.eval(64.0, 32.0);
        assert!(
            (center_val - f64::from(background)).abs() < 2.0,
            "model center: {center_val}, expected ~{background}"
        );
    }

    #[test]
    fn test_vertical_extremum_matches_pre_refactor_behavior() {
        // bd-xhqdo: lock the vertical min/max filter behavior so the Min/Max
        // zero-sized-type generic in vertical_extremum_filter cannot silently
        // drift from the pre-bd-xhqdo hand-written twins.
        //
        // With the symmetric ±half window (post-fix), kernel=3 (half=1):
        //   row:  0   1   2   3   4   5   6
        //   val: 10   2   8   1   9   3   7
        //
        // MIN over [c-1, c+1] intersected with [0, 6]:
        //   c=0: min(10, 2)             = 2
        //   c=1: min(10, 2, 8)          = 2
        //   c=2: min(2,  8, 1)          = 1
        //   c=3: min(8,  1, 9)          = 1
        //   c=4: min(1,  9, 3)          = 1
        //   c=5: min(9,  3, 7)          = 3
        //   c=6: min(3,  7)             = 3
        let frame: Vec<f32> = vec![10.0, 2.0, 8.0, 1.0, 9.0, 3.0, 7.0];
        let w = 1;
        let h = 7;
        let kernel = 3;

        let min_out = super::vertical_minimum_filter(&frame, w, h, kernel);
        let expected_min: [f32; 7] = [2.0, 2.0, 1.0, 1.0, 1.0, 3.0, 3.0];
        assert_eq!(min_out, expected_min, "vertical_minimum_filter");

        // MAX over [c-1, c+1] intersected with [0, 6]:
        //   c=0: max(10, 2)    = 10
        //   c=1: max(10, 2, 8) = 10
        //   c=2: max(2,  8, 1) = 8
        //   c=3: max(8,  1, 9) = 9
        //   c=4: max(1,  9, 3) = 9
        //   c=5: max(9,  3, 7) = 9
        //   c=6: max(3,  7)    = 7
        let max_out = super::vertical_maximum_filter(&frame, w, h, kernel);
        let expected_max: [f32; 7] = [10.0, 10.0, 8.0, 9.0, 9.0, 9.0, 7.0];
        assert_eq!(max_out, expected_max, "vertical_maximum_filter");
    }

    #[test]
    fn test_vertical_extremum_handles_nans_in_column() {
        // Non-finite values at the tail make the deque pop (the impl treats
        // !is_finite as "pop-eligible"); in the flush scan they are skipped.
        // Symmetric ±half window:
        //   row:  0     1      2    3    4
        //   val:  5     NaN    2    8    NaN
        //
        //   c=0 over [0,1]  = min(5, NaN)       = 5
        //   c=1 over [0,2]  = min(5, NaN, 2)    = 2
        //   c=2 over [1,3]  = min(NaN, 2, 8)    = 2
        //   c=3 over [2,4]  = min(2, 8, NaN)    = 2
        //   c=4 over [3,4]  = min(8, NaN)       = 8
        let frame: Vec<f32> = vec![5.0, f32::NAN, 2.0, 8.0, f32::NAN];
        let w = 1;
        let h = 5;
        let min_out = super::vertical_minimum_filter(&frame, w, h, 3);
        assert_eq!(min_out, vec![5.0, 2.0, 2.0, 2.0, 8.0]);
    }

    #[test]
    fn test_subtract_from_matches_naive_eval_per_pixel() {
        // bd-gn016: the cached `subtract_from` hot path must produce the same
        // per-pixel scatter value as a naive `for each px: model.eval(col,row)`
        // loop. Sub-ULP summation-order differences are expected and bounded
        // to a few ULPs per pixel against the signal.
        let width: u32 = 384;
        let height: u32 = 256;
        let model = ScatteredLightModel {
            // Pick non-trivial coefficients across the full (nx=3, ny=3) table
            // so every c_{iy,ix} term contributes and rounding drifts are
            // exercised rather than masked.
            coefficients: vec![
                100.0, 5.0, -0.5, // iy = 0
                20.0, -3.0, 0.25, // iy = 1
                -10.0, 2.5, 0.125, // iy = 2
            ],
            degree_x: 2,
            degree_y: 2,
            width,
            height,
        };

        // Start from a constant baseline so the corrected-pixel value isolates
        // (baseline − scatter) for straightforward comparison.
        let baseline = 1000.0_f32;
        let mut fast = vec![baseline; width as usize * height as usize];
        let mut naive = fast.clone();

        // Fast path (cached).
        model.subtract_from(&mut fast, width);

        // Naive reference.
        let w = width as usize;
        for (idx, pixel) in naive.iter_mut().enumerate() {
            let col = idx % w;
            let row = idx / w;
            let scatter = model.eval(col as f64, row as f64);
            *pixel = (f64::from(*pixel) - scatter).max(0.0) as f32;
        }

        let peak = naive.iter().copied().fold(0.0_f32, f32::max);
        // Tolerance: a few ULPs at f32 precision of the peak value, scaled
        // by the number of terms per sum (nx·ny = 9). 1e-4 relative comfortably
        // covers it while still catching any real algorithmic drift.
        let abs_tol = peak * 1e-4;
        let mut worst = 0.0_f32;
        for (i, (&f, &n)) in fast.iter().zip(naive.iter()).enumerate() {
            let d = (f - n).abs();
            worst = worst.max(d);
            assert!(
                d <= abs_tol,
                "pixel {i}: fast={f} naive={n} diff={d} tol={abs_tol}"
            );
        }
        // Assert the implementation is in fact close to bit-equal, not just
        // within the looser 1e-4 guard — drift larger than ~1e-8 · peak would
        // hint at a real algorithm change hiding behind a forgiving tolerance.
        assert!(
            worst < peak * 1e-6,
            "fast/naive diverged by {worst} on peak={peak} — stricter than expected tolerance"
        );
    }

    #[test]
    fn test_linear_gradient_scatter() {
        // Frame with a linear gradient: scatter = 10 + 0.5*x.
        let width: u32 = 200;
        let height: u32 = 100;
        let mut frame = Vec::with_capacity(width as usize * height as usize);
        for _row in 0..height {
            for col in 0..width {
                frame.push(10.0 + 0.5 * col as f32);
            }
        }

        let config = ScatteredLightConfig {
            block_size: 32,
            poly_degree_x: 2,
            poly_degree_y: 1,
            aperture_half_width: 3.0,
            ..ScatteredLightConfig::default()
        };

        let (corrected, _model) =
            subtract_scattered_light(&frame, width, height, &[], &config).expect("should fit");

        // After subtraction, residuals should be small.
        let max_residual = corrected.iter().copied().fold(0.0f32, f32::max);
        assert!(
            max_residual < 3.0,
            "max residual {max_residual} too large for linear gradient"
        );
    }

    #[test]
    fn test_scatter_with_orders_excluded() {
        // Frame with uniform scatter=20 in inter-order regions,
        // and bright signal=1000 on the order traces.
        let width: u32 = 200;
        let height: u32 = 100;
        let trace_center = 50.0;
        let trace = flat_trace(trace_center);
        let aperture = 5.0;

        let mut frame = vec![20.0f32; width as usize * height as usize];
        // Add bright signal along the trace.
        for col in 0..width as usize {
            for row_offset in -5i32..=5 {
                let row = (trace_center as i32 + row_offset) as usize;
                if row < height as usize {
                    frame[row * width as usize + col] = 1000.0;
                }
            }
        }

        let traces = [TraceInfo {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
        }];

        let config = ScatteredLightConfig {
            block_size: 32,
            poly_degree_x: 1,
            poly_degree_y: 1,
            aperture_half_width: aperture,
            ..ScatteredLightConfig::default()
        };

        let (_corrected, model) =
            subtract_scattered_light(&frame, width, height, &traces, &config).expect("should fit");

        // Model should estimate ~20 (the inter-order background), not ~1000.
        let inter_order_val = model.eval(100.0, 10.0);
        assert!(
            (inter_order_val - 20.0).abs() < 5.0,
            "inter-order scatter estimate: {inter_order_val}, expected ~20"
        );
    }

    #[test]
    fn test_2d_chebyshev_roundtrip() {
        // Fit a known bivariate polynomial and verify eval.
        let coeffs = vec![
            100.0, 5.0, 0.0, 0.0, // iy=0: c00=100, c10=5, c20=0, c30=0
            3.0, 0.0, 0.0, 0.0, // iy=1: c01=3
            0.0, 0.0, 0.0, 0.0, // iy=2
            0.0, 0.0, 0.0, 0.0, // iy=3
        ];
        // f(x,y) = 100 + 5*T_1(x) + 3*T_1(y) = 100 + 5x + 3y

        let val = eval_2d_chebyshev(&coeffs, 3, 3, 0.0, 0.0);
        assert!((val - 100.0).abs() < 1e-10, "at origin: {val}");

        let val = eval_2d_chebyshev(&coeffs, 3, 3, 1.0, 0.0);
        assert!((val - 105.0).abs() < 1e-10, "at (1,0): {val}");

        let val = eval_2d_chebyshev(&coeffs, 3, 3, 0.0, 1.0);
        assert!((val - 103.0).abs() < 1e-10, "at (0,1): {val}");
    }

    #[test]
    fn test_too_few_samples_returns_none() {
        // Very small frame with many orders → not enough inter-order pixels.
        let width: u32 = 10;
        let height: u32 = 10;
        let frame = vec![100.0f32; width as usize * height as usize];

        // Fill entire frame with orders (pre-create owned traces).
        let owned_traces: Vec<EchelleTraceModel> = (0..10)
            .map(|i| EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![f64::from(i)],
                domain_start: 0.0,
                domain_end: 100.0,
            })
            .collect();

        let traces: Vec<TraceInfo<'_>> = owned_traces
            .iter()
            .map(|t| TraceInfo {
                trace: t,
                disp_start: 0,
                disp_end: width - 1,
            })
            .collect();

        let config = ScatteredLightConfig {
            block_size: 5,
            poly_degree_x: 3,
            poly_degree_y: 3,
            ..Default::default()
        };

        // This may or may not return None depending on how many inter-order
        // pixels survive. The key is it doesn't panic.
        let _result = estimate_scattered_light(&frame, width, height, &traces, &config);
    }

    #[test]
    fn test_fast_scatter_uniform_background() {
        let width: u32 = 64;
        let height: u32 = 32;
        let background = 100.0f32;
        let frame = vec![background; width as usize * height as usize];

        let map = estimate_scatter_map_fast_f32(&frame, width, height, &[], 3.0, 8, 4);

        assert_eq!(map.len(), width as usize * height as usize);
        // With no traces, all pixels are inter-order → scatter ≈ background.
        for &v in &map {
            assert!(
                (v - background).abs() < 5.0,
                "scatter estimate {v} should be ~{background}"
            );
        }
    }

    #[test]
    fn test_fast_scatter_subtracts_background() {
        let width: u32 = 200;
        let height: u32 = 100;
        let background = 50.0f32;
        let trace_center = 50.0;
        let trace = flat_trace(trace_center);
        let signal = 1000.0f32;

        let mut frame = vec![background; width as usize * height as usize];
        // Add bright signal along the trace.
        for col in 0..width as usize {
            for offset in -5i32..=5 {
                let row = (trace_center as i32 + offset) as usize;
                if row < height as usize {
                    frame[row * width as usize + col] = signal;
                }
            }
        }

        let traces = [TraceInfo {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
        }];

        let mut corrected = frame.clone();
        subtract_scattered_light_fast_f32(&mut corrected, width, height, &traces, 6.0, 8, 4);

        // Inter-order pixels should be near zero after subtraction.
        let inter_order_val = corrected[10 * width as usize + 50]; // row 10, far from trace
        assert!(
            inter_order_val < 10.0,
            "inter-order pixel should be near 0, got {inter_order_val}"
        );

        // On-trace pixels should retain most of the signal.
        let on_trace_val = corrected[50 * width as usize + 100];
        assert!(
            on_trace_val > signal - background - 10.0,
            "on-trace pixel should retain signal, got {on_trace_val}"
        );
    }
}
