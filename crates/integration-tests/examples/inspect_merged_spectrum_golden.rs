//! Print descriptive stats for `debug/phase5/reference_merged_spectrum.hdf5`.
//!
//! Runs independently of the regression test so a human can spot-check the
//! golden before committing. Invoke from the workspace root with:
//!
//! ```
//! cargo run -p integration-tests --features storage_hdf5 \
//!     --example inspect_merged_spectrum_golden
//! ```
//!
//! bd-g22gu.4

use std::path::Path;

const GOLDEN_PATH: &str = "debug/phase5/reference_merged_spectrum.hdf5";

fn main() -> anyhow::Result<()> {
    let file = hdf5::File::open(Path::new(GOLDEN_PATH))?;
    let wl = file.dataset("wavelength_nm")?.read_raw::<f64>()?;
    let fx = file.dataset("flux")?.read_raw::<f64>()?;
    let vr = file.dataset("variance")?.read_raw::<f64>()?;
    let no = file.dataset("n_orders_per_bin")?.read_raw::<u32>()?;

    let n = wl.len();
    let dl = if n >= 2 { wl[1] - wl[0] } else { f64::NAN };
    let finite_fx: Vec<f64> = fx.iter().copied().filter(|f| f.is_finite()).collect();
    let peak = finite_fx.iter().copied().fold(0.0_f64, f64::max);
    let median = {
        let mut v = finite_fx.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() {
            f64::NAN
        } else {
            v[v.len() / 2]
        }
    };
    let max_orders = no.iter().copied().max().unwrap_or(0);
    let bins_covered = no.iter().filter(|&&x| x > 0).count();

    println!("golden: {GOLDEN_PATH}");
    println!("  n_bins             = {n}");
    println!(
        "  λ range            = [{:.4}, {:.4}] nm  (dλ = {:.4} nm)",
        wl[0],
        wl[n - 1],
        dl
    );
    println!("  finite flux bins   = {}/{n}", finite_fx.len());
    println!("  flux peak          = {peak:.3e}");
    println!("  flux median finite = {median:.3e}");
    println!(
        "  variance min/max   = {:.3e} / {:.3e}",
        vr.iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f64::INFINITY, f64::min),
        vr.iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f64::NEG_INFINITY, f64::max)
    );
    println!(
        "  n_orders_per_bin   = {{min: {}, max: {max_orders}, bins_with_coverage: {bins_covered}}}",
        no.iter().copied().min().unwrap_or(0)
    );

    // Attributes for provenance.
    for name in ["n_orders_calibrated", "n_orders_merged"] {
        let a = file.attr(name)?.read_scalar::<u32>()?;
        println!("  attr {name} = {a}");
    }
    for name in ["overall_rms_nm", "grid_spacing_nm"] {
        let a = file.attr(name)?.read_scalar::<f64>()?;
        println!("  attr {name} = {a:.6}");
    }
    Ok(())
}
