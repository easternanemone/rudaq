//! Colormap definitions and contrast/scale mode enums for image display.

/// Colormap for image display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Viridis,
    Inferno,
    Plasma,
    Magma,
}

impl Colormap {
    pub fn label(self) -> &'static str {
        match self {
            Self::Grayscale => "Grayscale",
            Self::Viridis => "Viridis",
            Self::Inferno => "Inferno",
            Self::Plasma => "Plasma",
            Self::Magma => "Magma",
        }
    }

    /// Apply colormap to a normalized value (0.0-1.0) returning RGB
    /// Uses pre-computed LUT for performance (bd-7rk0)
    #[inline]
    pub fn apply(self, value: f32) -> [u8; 3] {
        // Convert to 8-bit index (0-255)
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let idx = (value.clamp(0.0, 1.0) * 255.0) as usize;
        self.lut()[idx]
    }

    /// Get the pre-computed LUT for this colormap (256 RGB entries)
    #[inline]
    fn lut(self) -> &'static [[u8; 3]; 256] {
        match self {
            Self::Grayscale => &GRAYSCALE_LUT,
            Self::Viridis => &VIRIDIS_LUT,
            Self::Inferno => &INFERNO_LUT,
            Self::Plasma => &PLASMA_LUT,
            Self::Magma => &MAGMA_LUT,
        }
    }
}

// Implement ColormapTrait for Colorbar widget (bd-07j1)
impl crate::widgets::ColormapTrait for Colormap {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn apply(&self, value: f32) -> [u8; 3] {
        Colormap::apply(*self, value)
    }
}

// Pre-computed colormap lookup tables (bd-7rk0: performance optimization)
// Each LUT has 256 entries for O(1) intensity-to-color mapping

#[allow(clippy::cast_possible_truncation)]
static GRAYSCALE_LUT: [[u8; 3]; 256] = {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        lut[i] = [i as u8, i as u8, i as u8];
        i += 1;
    }
    lut
};

static VIRIDIS_LUT: [[u8; 3]; 256] = compute_viridis_lut();
static INFERNO_LUT: [[u8; 3]; 256] = compute_inferno_lut();
static PLASMA_LUT: [[u8; 3]; 256] = compute_plasma_lut();
static MAGMA_LUT: [[u8; 3]; 256] = compute_magma_lut();

#[allow(clippy::many_single_char_names)]
const fn compute_viridis_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        #[allow(clippy::cast_precision_loss)]
        let v = i as f64 / 255.0;
        // Viridis: purple -> blue -> green -> yellow
        let r = (0.267 + v * (0.993 - 0.267)) * 255.0;
        let g = v * 0.906 * 255.0;
        let b = (0.329 + v * 0.186) * 255.0; // Simplified for const fn
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

#[allow(clippy::many_single_char_names)]
const fn compute_inferno_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        #[allow(clippy::cast_precision_loss)]
        let v = i as f64 / 255.0;
        // Inferno: black -> purple -> red -> yellow (using sqrt/pow approximations)
        let r = const_sqrt(v) * 255.0;
        let g = v * v * v * 200.0; // powf(1.5) approximated
        let b = (1.0 - v) * v * 4.0 * 255.0;
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

#[allow(clippy::many_single_char_names)]
const fn compute_plasma_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        #[allow(clippy::cast_precision_loss)]
        let v = i as f64 / 255.0;
        // Plasma: blue -> purple -> orange -> yellow
        let r = (0.05 + v * 0.95) * 255.0;
        let g = v * v * 255.0;
        let b = (1.0 - v * 0.7) * 255.0;
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

#[allow(clippy::many_single_char_names)]
const fn compute_magma_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    let mut i = 0;
    while i < 256 {
        #[allow(clippy::cast_precision_loss)]
        let v = i as f64 / 255.0;
        // Magma: black -> purple -> pink -> white
        let r = const_pow_0_7(v) * 255.0;
        let g = v * v * 200.0;
        let b = (0.3 + v * 0.7) * v * 255.0;
        lut[i] = [clamp_u8(r), clamp_u8(g), clamp_u8(b)];
        i += 1;
    }
    lut
}

/// Clamp f64 to u8 range (const fn compatible)
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn clamp_u8(v: f64) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v as u8
    }
}

/// Const-compatible sqrt approximation using Newton-Raphson
const fn const_sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = x / 2.0;
    let mut i = 0;
    while i < 10 {
        guess = f64::midpoint(guess, x / guess);
        i += 1;
    }
    guess
}

/// Const-compatible x^0.7 approximation
const fn const_pow_0_7(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    // x^0.7 ≈ x * x^(-0.3) ≈ x / x^0.3
    // Using sqrt approximation: x^0.5 then interpolate
    let sqrt_x = const_sqrt(x);
    // x^0.7 ≈ sqrt(x) * x^0.2 ≈ sqrt(x) * sqrt(sqrt(x))^0.4
    // Simplified: use linear interpolation between x and sqrt(x)
    // x^0.7 ≈ 0.4*x + 0.6*sqrt(x) (rough approximation)
    sqrt_x * 0.7 + x * 0.3
}

/// Contrast enhancement mode (bd-j6xm)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContrastMode {
    /// Manual min/max control
    #[default]
    Manual,
    /// Simple min/max from all pixels
    AutoSimple,
    /// Percentile-based (ignore outliers)
    AutoPercentile,
    /// Histogram equalization
    HistogramEq,
    /// Contrast Limited Adaptive Histogram Equalization
    Clahe,
}

impl ContrastMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::AutoSimple => "Auto (Simple)",
            Self::AutoPercentile => "Auto (Percentile)",
            Self::HistogramEq => "Histogram Eq",
            Self::Clahe => "CLAHE",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Manual,
            Self::AutoSimple,
            Self::AutoPercentile,
            Self::HistogramEq,
            Self::Clahe,
        ]
    }
}

/// Scale mode for pixel intensity mapping
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaleMode {
    #[default]
    Linear,
    Log,
    Sqrt,
}

impl ScaleMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Log => "Log",
            Self::Sqrt => "Sqrt",
        }
    }

    /// Apply scaling to a normalized value (0.0-1.0)
    pub fn apply(self, value: f32) -> f32 {
        match self {
            Self::Linear => value,
            Self::Log => (1.0 + value * 99.0).log10() / 2.0, // log10(1-100) -> 0-2 -> 0-1
            Self::Sqrt => value.sqrt(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_u8() {
        assert_eq!(clamp_u8(-10.0), 0);
        assert_eq!(clamp_u8(0.0), 0);
        assert_eq!(clamp_u8(127.5), 127);
        assert_eq!(clamp_u8(255.0), 255);
        assert_eq!(clamp_u8(300.0), 255);
    }

    #[test]
    fn test_const_sqrt() {
        assert_eq!(const_sqrt(0.0), 0.0);
        assert!((const_sqrt(1.0) - 1.0).abs() < 0.01);
        assert!((const_sqrt(4.0) - 2.0).abs() < 0.01);
        assert!((const_sqrt(0.25) - 0.5).abs() < 0.01);
    }
}
