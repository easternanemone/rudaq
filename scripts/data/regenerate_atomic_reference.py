#!/usr/bin/env python3
"""Regenerate crates/atomic-reference/src/lines_data.rs from the CF-LIBS NIST ASD DB.

Consumes the sibling project's curated atomic-line database at
`~/code/CF-LIBS-improved/ASD_da/libs_production.db` (see
`docs/design/nist-asd-integration-decision.md`) and emits a Rust source
file with `pub const LINES: &[AtomicLine] = &[...]` entries filtered to
the species and wavelength range rust-daq's wavelength-calibration
pipeline uses.

Usage:
    uv run scripts/data/regenerate_atomic_reference.py
    # or
    python3 scripts/data/regenerate_atomic_reference.py

Options (environment variables):
    CFLIBS_DB_PATH   Override the path to the CF-LIBS SQLite file.
                     Default: ~/code/CF-LIBS-improved/ASD_da/libs_production.db

The generated file is committed to the repository; this script is run
by developers when CF-LIBS's atomic data is updated, not on every build.
"""
from __future__ import annotations

import datetime
import os
import pathlib
import sqlite3
import sys
import textwrap

# ─── Configuration ──────────────────────────────────────────────────────────

DEFAULT_DB_PATH = pathlib.Path.home() / "code" / "CF-LIBS-improved" / "ASD_da" / "libs_production.db"
OUTPUT_PATH = pathlib.Path(__file__).resolve().parents[2] / "crates" / "atomic-reference" / "src" / "lines_data.rs"

# Mechelle 5000 detector bandpass. Lines outside this range are never
# visible on the iStar, so filtering here keeps the generated file lean.
WAVELENGTH_MIN_NM = 200.0
WAVELENGTH_MAX_NM = 975.0

# Species rust-daq cares about for wavelength calibration. Th is kept
# against future ThAr hollow-cathode lamp acquisition (memo-dtw-line-id.md
# §5 trigger condition). Ne is kept for HgNe variants of Ocean Optics lamps.
# Expanding this list is cheap — rerun the script.
INCLUDED_SPECIES = {
    ("Hg", 1),
    ("Hg", 2),
    ("Ar", 1),
    ("Ar", 2),
    ("Ne", 1),
    ("Th", 1),
    ("Th", 2),
}


def roman(sp_num: int) -> str:
    """Roman-numeral ionization notation used by NIST ASD (I, II, III, ...)."""
    numerals = {1: "I", 2: "II", 3: "III", 4: "IV", 5: "V", 6: "VI"}
    return numerals.get(sp_num, str(sp_num))


def fetch_lines(db_path: pathlib.Path) -> list[dict]:
    if not db_path.exists():
        print(f"ERROR: {db_path} not found.", file=sys.stderr)
        print(
            "Set CFLIBS_DB_PATH or ensure CF-LIBS-improved is checked out at ~/code/.",
            file=sys.stderr,
        )
        sys.exit(2)

    con = sqlite3.connect(db_path)
    con.row_factory = sqlite3.Row

    rows: list[dict] = []
    for element, sp_num in sorted(INCLUDED_SPECIES):
        cursor = con.execute(
            """
            SELECT wavelength_nm, element, sp_num, aki,
                   ei_ev, ek_ev, gi, gk, accuracy_grade
            FROM lines
            WHERE element = ?
              AND sp_num = ?
              AND wavelength_nm BETWEEN ? AND ?
            ORDER BY wavelength_nm ASC
            """,
            (element, sp_num, WAVELENGTH_MIN_NM, WAVELENGTH_MAX_NM),
        )
        for row in cursor:
            rows.append(
                {
                    "wavelength_nm": float(row["wavelength_nm"]),
                    "element": row["element"],
                    "sp_num": int(row["sp_num"]),
                    "aki": float(row["aki"]) if row["aki"] is not None else 0.0,
                    "ei_ev": float(row["ei_ev"]) if row["ei_ev"] is not None else 0.0,
                    "ek_ev": float(row["ek_ev"]) if row["ek_ev"] is not None else 0.0,
                    "gi": int(row["gi"]) if row["gi"] is not None else 0,
                    "gk": int(row["gk"]) if row["gk"] is not None else 0,
                    "accuracy_grade": row["accuracy_grade"] if row["accuracy_grade"] is not None else "",
                }
            )
    con.close()
    return rows


def render_rust_literal(val) -> str:
    """Render a Python value as a Rust literal WITHOUT a type suffix.

    Rust infers the type from the struct field definition, so suffixes
    (`_u8`, `_u32`, `_f64`) would only risk mismatches when field types
    are changed. Floats use a fixed-precision format to keep diffs
    reproducible across Python versions.
    """
    if isinstance(val, bool):
        # Must come before the int branch — Python bools are ints.
        return "true" if val else "false"
    if isinstance(val, float):
        return f"{val:.6}"
    if isinstance(val, int):
        return str(val)
    if isinstance(val, str):
        # Escape quotes and backslashes — NIST species strings are ASCII,
        # but accuracy grades like "B+" / "AA" etc. are quote-safe too.
        escaped = val.replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'
    raise TypeError(f"unsupported literal type: {type(val)}")


def render_rust_file(rows: list[dict], db_path: pathlib.Path) -> str:
    # Deterministic ordering by (element, sp_num, wavelength) so every run
    # produces the same file — clean git diffs between regenerations.
    rows = sorted(rows, key=lambda r: (r["element"], r["sp_num"], r["wavelength_nm"]))

    # Generation metadata (fixed parts only — no wall-clock timestamp,
    # which would cause spurious diffs on every regen).
    header = textwrap.dedent(
        f"""\
        // GENERATED FILE — do not edit by hand.
        //
        // Produced by: scripts/data/regenerate_atomic_reference.py
        // Source:       {db_path.name} (CF-LIBS-improved NIST ASD database)
        // Species:      {", ".join(f"{el} {roman(sp)}" for el, sp in sorted(INCLUDED_SPECIES))}
        // λ range:      {WAVELENGTH_MIN_NM} nm – {WAVELENGTH_MAX_NM} nm
        //
        // To regenerate, run:
        //     uv run scripts/data/regenerate_atomic_reference.py
        //
        // See docs/design/nist-asd-integration-decision.md for rationale.

        use crate::AtomicLine;

        /// Atomic emission lines from NIST ASD, filtered to species and
        /// wavelengths relevant to rust-daq's echelle calibration pipeline.
        ///
        /// Ordered by `(element, sp_num, wavelength_nm)` for stable diffs on
        /// regeneration. See [`crate::lines_for_element`] and
        /// [`crate::atlas::hgar`] for queryable accessors.
        pub const LINES: &[AtomicLine] = &[
        """
    )

    lines_out: list[str] = [header]
    for r in rows:
        entry = (
            f"    AtomicLine {{ "
            f"wavelength_nm: {render_rust_literal(r['wavelength_nm'])}, "
            f"element: {render_rust_literal(r['element'])}, "
            f"sp_num: {render_rust_literal(r['sp_num'])}, "
            f"aki: {render_rust_literal(r['aki'])}, "
            f"ei_ev: {render_rust_literal(r['ei_ev'])}, "
            f"ek_ev: {render_rust_literal(r['ek_ev'])}, "
            f"gi: {render_rust_literal(r['gi'])}, "
            f"gk: {render_rust_literal(r['gk'])}, "
            f"accuracy_grade: {render_rust_literal(r['accuracy_grade'])} "
            f"}},"
        )
        lines_out.append(entry)
    lines_out.append("];")
    lines_out.append("")
    return "\n".join(lines_out)


def main() -> int:
    db_path = pathlib.Path(os.environ.get("CFLIBS_DB_PATH", str(DEFAULT_DB_PATH))).expanduser()
    print(f"Reading {db_path}...")
    rows = fetch_lines(db_path)
    rendered = render_rust_file(rows, db_path)

    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_text(rendered)

    # Per-species counts for the human running this script.
    counts: dict[tuple[str, int], int] = {}
    for r in rows:
        key = (r["element"], r["sp_num"])
        counts[key] = counts.get(key, 0) + 1
    print(f"Wrote {len(rows)} lines to {OUTPUT_PATH.relative_to(OUTPUT_PATH.parents[2])}")
    for (el, sp), c in sorted(counts.items()):
        print(f"  {el} {roman(sp):>4}: {c} lines")
    return 0


if __name__ == "__main__":
    sys.exit(main())
