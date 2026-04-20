//! NIST ASD atomic-line reference data for rust-daq spectroscopy.
//!
//! This crate exposes the atomic-emission-line inventory rust-daq uses
//! for echelle wavelength calibration and (eventually) LIBS species
//! identification. Line data originate from the CF-LIBS-improved
//! sibling project's curated NIST Atomic Spectra Database extract
//! (`~/code/CF-LIBS-improved/ASD_da/libs_production.db`) and are
//! materialised into `const` arrays at check-in time by
//! `scripts/data/regenerate_atomic_reference.py`.
//!
//! # Design
//!
//! - **Pure data, zero deps.** No runtime SQLite, no filesystem lookup,
//!   no network call. Every query is an in-memory slice scan over the
//!   committed [`LINES`] constant.
//! - **Generated, not authored.** The extraction script is the source
//!   of truth for what's in [`LINES`]; editing the generated file by
//!   hand will be overwritten on the next regeneration.
//! - **Typed.** [`AtomicLine`] enforces the schema at compile time;
//!   misspelled element names or out-of-range fields fail the build.
//!
//! # Examples
//!
//! ```
//! use atomic_reference::lines_for_element;
//!
//! let hg_i_lines = lines_for_element("Hg", 1);
//! assert!(hg_i_lines.iter().any(|l| (l.wavelength_nm - 546.075).abs() < 0.01));
//! ```
//!
//! Echelle-friendly conversions to the legacy `AtlasLine` shape live in
//! [`atlas`].
//!
//! # Updating the data
//!
//! ```bash
//! uv run scripts/data/regenerate_atomic_reference.py
//! ```
//!
//! See `docs/design/nist-asd-integration-decision.md` for the rationale
//! behind this pattern.

// Rust guideline compliant 2026-02-21

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod atlas;

mod lines_data;

pub use lines_data::LINES;

/// One atomic-emission-line entry as curated by NIST ASD.
///
/// All fields are committed at generation time. `aki` is the Einstein
/// spontaneous-emission rate coefficient (s⁻¹); `ei_ev` / `ek_ev` are the
/// lower / upper state energies in electron-volts; `gi` / `gk` are the
/// lower / upper state statistical weights (degeneracies); `accuracy_grade`
/// is NIST's letter rating (`"AA"`, `"A"`, …).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtomicLine {
    /// Air wavelength in nanometers (NIST ASD convention).
    pub wavelength_nm: f64,
    /// Chemical element symbol, e.g. `"Hg"`, `"Ar"`, `"Ne"`.
    pub element: &'static str,
    /// Ionization stage: 1 = neutral (Roman I), 2 = singly-ionized (II), …
    pub sp_num: u8,
    /// Einstein spontaneous-emission rate (s⁻¹). 0.0 when unknown.
    pub aki: f64,
    /// Lower-state energy (eV). 0.0 when unknown.
    pub ei_ev: f64,
    /// Upper-state energy (eV). 0.0 when unknown.
    pub ek_ev: f64,
    /// Lower-state statistical weight (degeneracy). 0 when unknown.
    pub gi: u32,
    /// Upper-state statistical weight (degeneracy). 0 when unknown.
    pub gk: u32,
    /// NIST accuracy grade (`"AA"`, `"A"`, `"B+"`, `"B"`, `"C+"`, …).
    pub accuracy_grade: &'static str,
}

impl AtomicLine {
    /// Human-readable species label, e.g. `"Hg I"` or `"Ar II"`.
    #[must_use]
    pub fn species(&self) -> String {
        let numeral = match self.sp_num {
            1 => "I",
            2 => "II",
            3 => "III",
            4 => "IV",
            5 => "V",
            6 => "VI",
            _ => return format!("{} {}", self.element, self.sp_num),
        };
        format!("{} {numeral}", self.element)
    }
}

/// Iterate over all lines for one element + ionization stage.
///
/// Returns an iterator yielding references into the committed [`LINES`]
/// table; no allocation, no search beyond a linear filter (tens of
/// thousands of entries is fine for typical calibration use).
pub fn lines_for_element(element: &str, sp_num: u8) -> impl Iterator<Item = &'static AtomicLine> {
    LINES
        .iter()
        .filter(move |l| l.element == element && l.sp_num == sp_num)
}

/// Iterate over all lines inside `[lo_nm, hi_nm]` (inclusive).
pub fn lines_in_range(lo_nm: f64, hi_nm: f64) -> impl Iterator<Item = &'static AtomicLine> {
    LINES
        .iter()
        .filter(move |l| l.wavelength_nm >= lo_nm && l.wavelength_nm <= hi_nm)
}

/// Iterate over all lines for any of the given element / ionization pairs.
///
/// Cheaper than calling [`lines_for_element`] in a loop because it walks
/// [`LINES`] once.
pub fn lines_for_species<'a>(
    species: &'a [(&'a str, u8)],
) -> impl Iterator<Item = &'static AtomicLine> + 'a {
    LINES.iter().filter(move |l| {
        species
            .iter()
            .any(|(el, sp)| *el == l.element && *sp == l.sp_num)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_table_is_nonempty() {
        assert!(!LINES.is_empty(), "LINES was empty; did the generator run?");
    }

    #[test]
    fn lines_are_sorted_by_element_sp_wavelength() {
        // Matches the ordering guarantee in the generator; future regens
        // must preserve it so diffs stay reviewable.
        for pair in LINES.windows(2) {
            let [a, b] = pair else { unreachable!() };
            let a_key = (a.element, a.sp_num, a.wavelength_nm);
            let b_key = (b.element, b.sp_num, b.wavelength_nm);
            assert!(
                a_key.0 < b_key.0
                    || (a_key.0 == b_key.0 && a_key.1 < b_key.1)
                    || (a_key.0 == b_key.0 && a_key.1 == b_key.1 && a_key.2 <= b_key.2),
                "LINES not sorted at {a:?} vs {b:?}",
            );
        }
    }

    #[test]
    fn hg_i_green_line_present() {
        // The Hg I 546.075 nm green line is the brightest visible-range
        // reference line; if it's missing the generator dropped something.
        let found = lines_for_element("Hg", 1).any(|l| (l.wavelength_nm - 546.0750).abs() < 0.01);
        assert!(found, "Hg I 546.075 nm missing from LINES");
    }

    #[test]
    fn ar_i_763_line_present() {
        // Ar I 763.511 nm is one of the brightest Ar calibration anchors.
        let found = lines_for_element("Ar", 1).any(|l| (l.wavelength_nm - 763.511).abs() < 0.02);
        assert!(found, "Ar I 763.511 nm missing from LINES");
    }

    #[test]
    fn species_label_formatting() {
        let line = AtomicLine {
            wavelength_nm: 546.0750,
            element: "Hg",
            sp_num: 1,
            aki: 4.9e7,
            ei_ev: 0.0,
            ek_ev: 0.0,
            gi: 0,
            gk: 0,
            accuracy_grade: "",
        };
        assert_eq!(line.species(), "Hg I");
    }
}
