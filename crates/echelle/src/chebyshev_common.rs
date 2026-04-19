//! Shared Chebyshev-polynomial primitives used across the echelle crate.
//!
//! The same `T_k(x) = 2x·T_{k-1}(x) − T_{k-2}(x)` recurrence was previously
//! implemented three times (wavelength_fitting, chebyshev_2d, scattered_light)
//! with slightly different parameter names (`n` = count vs. `degree` = max index).
//! This module centralises that recurrence plus the tensor-product 2D evaluator
//! so that perf work on the basis (e.g. bd-gn016's per-row/column caching) has
//! exactly one place to touch.
//!
//! All functions here are `pub(crate)`: they are implementation details of
//! `echelle`, not part of its public API surface.

/// Compute Chebyshev basis values `[T_0(x), T_1(x), …, T_{n-1}(x)]` via the
/// three-term recurrence `T_k = 2x·T_{k-1} − T_{k-2}` seeded with `T_0 = 1`
/// and `T_1 = x`.
///
/// `n` is the number of basis functions requested (i.e. `degree + 1`). Returns
/// an empty vector for `n == 0`.
#[must_use]
pub(crate) fn chebyshev_basis(x: f64, n: usize) -> Vec<f64> {
    let mut t = vec![0.0; n];
    chebyshev_basis_into(x, &mut t);
    t
}

/// Caller-owned-buffer variant of [`chebyshev_basis`].
///
/// Writes `T_0(x) .. T_{out.len()-1}(x)` into `out` in place. The caller is
/// responsible for the buffer length; any extra trailing capacity is left at
/// whatever value the caller initialised it to.
///
/// This is the primitive bd-gn016 uses to cache the per-row / per-column
/// basis when evaluating a scattered-light surface on every pixel of a
/// 2560×2160 frame — one pre-sized buffer per spatial axis replaces 5.5 M
/// short-lived allocations.
pub(crate) fn chebyshev_basis_into(x: f64, out: &mut [f64]) {
    let n = out.len();
    if n == 0 {
        return;
    }
    out[0] = 1.0;
    if n == 1 {
        return;
    }
    out[1] = x;
    for k in 2..n {
        out[k] = 2.0 * x * out[k - 1] - out[k - 2];
    }
}

/// Evaluate a 2D Chebyshev tensor-product surface at normalised coordinates
/// `(outer_norm, inner_norm)` against row-major coefficients `coeffs[o·n_inner + i]`.
///
/// The "outer" axis is the one that varies slowest in memory (larger stride).
/// Callers choose the mapping according to their stored layout:
///
/// - `coeffs[ix * nm + jm]` (x outer, m inner) — pass `(nx, nm, x_norm, m_norm)`.
/// - `coeffs[iy * nx + ix]` (y outer, x inner) — pass `(ny, nx, y_norm, x_norm)`.
///
/// Both layouts are in use in the crate; this helper accepts either by
/// parameter ordering alone, so no transpose or re-layout of existing
/// coefficient buffers is required.
#[must_use]
pub(crate) fn eval_2d(
    coeffs: &[f64],
    n_outer: usize,
    n_inner: usize,
    outer_norm: f64,
    inner_norm: f64,
) -> f64 {
    let t_outer = chebyshev_basis(outer_norm, n_outer);
    let t_inner = chebyshev_basis(inner_norm, n_inner);
    let mut result = 0.0;
    for (o, &outer) in t_outer.iter().enumerate() {
        let row_base = o * n_inner;
        for (i, &inner) in t_inner.iter().enumerate() {
            result += coeffs[row_base + i] * outer * inner;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from the closed-form Chebyshev polynomials evaluated at x = 0.5:
    //   T_0 = 1, T_1 = 0.5, T_2 = -0.5, T_3 = -1.0, T_4 = -0.5, T_5 = 0.5
    const X: f64 = 0.5;
    const BASIS_AT_HALF: [f64; 6] = [1.0, 0.5, -0.5, -1.0, -0.5, 0.5];

    #[test]
    fn chebyshev_basis_matches_closed_form() {
        let t = chebyshev_basis(X, 6);
        for (i, &expected) in BASIS_AT_HALF.iter().enumerate() {
            assert!(
                (t[i] - expected).abs() < 1e-14,
                "T_{i}({X}) = {} want {expected}",
                t[i]
            );
        }
    }

    #[test]
    fn chebyshev_basis_handles_edge_sizes() {
        assert!(chebyshev_basis(0.5, 0).is_empty());
        assert_eq!(chebyshev_basis(0.5, 1), vec![1.0]);
        assert_eq!(chebyshev_basis(0.5, 2), vec![1.0, 0.5]);
    }

    #[test]
    fn chebyshev_basis_into_writes_identical_values() {
        let out_alloc = chebyshev_basis(X, 6);
        let mut out_inplace = [0.0_f64; 6];
        chebyshev_basis_into(X, &mut out_inplace);
        assert_eq!(out_alloc, out_inplace.to_vec());
    }

    #[test]
    fn chebyshev_basis_into_handles_empty_buffer() {
        let mut out: [f64; 0] = [];
        chebyshev_basis_into(0.3, &mut out); // must not panic
    }

    #[test]
    fn eval_2d_reduces_to_outer_times_inner_on_pure_products() {
        // Coefficients all zero except c_{o=2, i=1} = 3.0.
        // Expected value: 3.0 · T_2(x) · T_1(y).
        let (n_outer, n_inner) = (3, 2);
        let mut coeffs = vec![0.0; n_outer * n_inner];
        coeffs[2 * n_inner + 1] = 3.0;
        let got = eval_2d(&coeffs, n_outer, n_inner, 0.5, 0.7);
        let want = 3.0 * BASIS_AT_HALF[2] * 0.7;
        assert!((got - want).abs() < 1e-14, "eval_2d {got} vs want {want}");
    }

    #[test]
    fn eval_2d_respects_caller_layout_choice() {
        // Same underlying data viewed two ways produces different sums unless
        // the caller matches n_outer/n_inner to the storage layout. This test
        // pins down the contract by constructing identical tensor products
        // in both layouts and checking they match.
        let (nx, nm) = (3, 2);
        // Reference: c_{ix=1, jm=0} = 5.0 in (x-outer, m-inner) layout.
        let mut row_major_x = vec![0.0; nx * nm];
        row_major_x[nm] = 5.0; // ix=1, jm=0 -> index 1*nm + 0
        // Same coefficient in (m-outer, x-inner) layout: jm=0 (outer) is index 0*nx + 1.
        let mut row_major_m = vec![0.0; nm * nx];
        row_major_m[1] = 5.0; // jm=0, ix=1 -> index 0*nx + 1
        let a = eval_2d(&row_major_x, nx, nm, 0.3, 0.4);
        let b = eval_2d(&row_major_m, nm, nx, 0.4, 0.3);
        assert!((a - b).abs() < 1e-14);
        // Both reduce to 5 · T_1(0.3) · T_0(0.4) = 5 · 0.3 · 1.0 = 1.5.
        assert!((a - 1.5).abs() < 1e-14);
    }
}
