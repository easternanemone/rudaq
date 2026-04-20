//! Echelle-calibration convenience views over [`super::LINES`].
//!
//! Echelle wavelength calibration in `crates/echelle` uses a narrow
//! `AtlasLine { wavelength_nm, species, strength }` shape. This module
//! produces that shape from the richer [`crate::AtomicLine`] entries
//! without forcing the echelle crate to depend on this crate's full
//! schema.

use crate::AtomicLine;

/// Echelle-pipeline atlas entry: wavelength, species label, strength.
///
/// Mirrors the shape of `echelle::wavelength_fitting::AtlasLine` without
/// creating a circular dependency. The echelle crate converts this into
/// its local type via a trivial field-by-field copy.
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasEntry {
    /// Air wavelength in nanometers.
    pub wavelength_nm: f64,
    /// Human-readable species label, e.g. `"Hg I"`.
    pub species: String,
    /// Relative strength used by the matcher's weighting logic.
    ///
    /// Currently derived from NIST `aki` (Einstein spontaneous-emission
    /// rate, s⁻¹) because `aki` is well-populated and scales
    /// monotonically with observed line brightness for isolated lines.
    /// Callers that need the raw `aki` should query the underlying
    /// [`crate::AtomicLine`] directly.
    pub strength: f64,
}

impl From<&AtomicLine> for AtlasEntry {
    fn from(line: &AtomicLine) -> Self {
        Self {
            wavelength_nm: line.wavelength_nm,
            species: line.species(),
            strength: normalize_strength(line.aki),
        }
    }
}

/// Map NIST Einstein `aki` (10³–10⁹ s⁻¹ for optical lines) into the
/// relative-intensity band used by the legacy hand-curated atlas
/// (~20–10 000). This preserves two contracts that are easy to break:
///
/// - `echelle::simulation::simulate_arc` synthesises per-line peak flux
///   as `emission_peak_flux × (strength / 1000.0)`. That divisor was
///   calibrated for a strength range of O(10²–10⁴); feeding raw `aki`
///   (up to ~10⁸) would over-brighten synthetic frames by a factor of
///   ~10 000 and saturate every peak.
/// - The matcher itself (`wavelength_fitting.rs`) currently does not
///   use `strength` for weighting, so this mapping does not affect
///   wavelength-calibration correctness on real frames. It only keeps
///   the simulation path honest.
///
/// The mapping is `max(1, min(aki / 1e5, 10_000))`: preserves the
/// ordering of line brightnesses, clips the bottom of the range at 1
/// (still non-zero so the line isn't silently dropped from simulation),
/// and clips the top at 10 000 to match the legacy scale. Lines with
/// unknown `aki` (stored as 0.0 by the generator) land at 1.0.
fn normalize_strength(aki: f64) -> f64 {
    if !aki.is_finite() || aki <= 0.0 {
        return 1.0;
    }
    (aki / 1e5).clamp(1.0, 10_000.0)
}

/// HgAr atlas for ME5000-class echelle calibration (Hg I + Ar I in
/// 200–975 nm, NIST ASD strong-line inventory).
///
/// Ordered by wavelength ascending.
#[must_use]
pub fn hgar() -> Vec<AtlasEntry> {
    let mut entries: Vec<AtlasEntry> = crate::LINES
        .iter()
        .filter(|l| matches!((l.element, l.sp_num), ("Hg", 1) | ("Ar", 1)))
        .map(AtlasEntry::from)
        .collect();
    entries.sort_by(|a, b| {
        a.wavelength_nm
            .partial_cmp(&b.wavelength_nm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

/// Pure-Mercury atlas for HG-2 type lamps (Hg I only, 200–975 nm).
#[must_use]
pub fn hg() -> Vec<AtlasEntry> {
    let mut entries: Vec<AtlasEntry> = crate::LINES
        .iter()
        .filter(|l| l.element == "Hg" && l.sp_num == 1)
        .map(AtlasEntry::from)
        .collect();
    entries.sort_by(|a, b| {
        a.wavelength_nm
            .partial_cmp(&b.wavelength_nm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

/// HgAr + neutral neon atlas for HG-2/Ne variants.
#[must_use]
pub fn hgar_ne() -> Vec<AtlasEntry> {
    let mut entries: Vec<AtlasEntry> = crate::LINES
        .iter()
        .filter(|l| matches!((l.element, l.sp_num), ("Hg", 1) | ("Ar", 1) | ("Ne", 1)))
        .map(AtlasEntry::from)
        .collect();
    entries.sort_by(|a, b| {
        a.wavelength_nm
            .partial_cmp(&b.wavelength_nm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hgar_spans_the_mechelle_5000_bandpass() {
        let atlas = hgar();
        assert!(!atlas.is_empty());
        let lo = atlas.first().unwrap().wavelength_nm;
        let hi = atlas.last().unwrap().wavelength_nm;
        assert!(lo >= 200.0 && hi <= 975.0);
        // We expect at least an order of magnitude more lines than the
        // legacy hand-curated 29-line atlas.
        assert!(
            atlas.len() >= 100,
            "hgar atlas only has {} lines; expansion not wired?",
            atlas.len()
        );
    }

    #[test]
    fn hgar_is_wavelength_sorted() {
        let atlas = hgar();
        for pair in atlas.windows(2) {
            let [a, b] = pair else { unreachable!() };
            assert!(a.wavelength_nm <= b.wavelength_nm);
        }
    }
}
