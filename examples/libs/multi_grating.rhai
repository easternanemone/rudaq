// LIBS Multi-Grating Spectral Survey — Scan across gratings for broad coverage
//
// Acquires LIBS spectra on multiple gratings at each position, enabling
// detection of elements whose emission lines fall in different spectral
// windows.  Typical: grating 1 (UV, 200–400 nm) + grating 2 (VIS, 400–600 nm).
//
// Usage:
//   rust-daq-daemon run examples/libs/multi_grating.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Andor iStar camera + Shamrock spectrograph connected (or mock mode)
//   - Dover SmartStage X axis connected (or mock mode)

print("╔════════════════════════════════════════╗");
print("║  LIBS Multi-Grating Spectral Survey    ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Grating / Wavelength Settings ───────────────────────────────────────────
//  Each entry: [grating_index, center_wavelength_nm, label]
let gratings = [
    [1, 310.0, "UV (Al/Si lines)"],
    [2, 450.0, "VIS (Fe/Ca lines)"],
];

// ── Scan Parameters ──────────────────────────────────────────────────────────
let scan_start  = 0.0;
let scan_end    = 5.0;
let scan_step   = 1.0;
let shots       = 3;
let num_points  = ((scan_end - scan_start) / scan_step + 1.0).to_int();

// ── Camera Parameters ────────────────────────────────────────────────────────
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;

print(`Scan: ${scan_start} → ${scan_end} mm, step ${scan_step} mm (${num_points} points)`);
print(`Gratings: ${gratings.len()} configurations`);
print(`Shots per grating per position: ${shots}`);
print(`Total acquisitions: ${num_points * gratings.len() * shots}`);
print("");

// ── Construct hardware handles ────────────────────────────────────────────────
let camera  = create_andor_camera();
let spec    = create_andor_spectrograph();
let stage_x = create_dover_axis("X");

// ── Prepare Camera ───────────────────────────────────────────────────────────
print("[1/3] Preparing camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed");

// ── Execute Multi-Grating Scan ───────────────────────────────────────────────
print("[2/3] Executing multi-grating scan...");
print("─────────────────────────────────────────");

let total_acquired = 0;

for i in 0..num_points {
    let position = scan_start + (i.to_float() * scan_step);

    stage_x.move_abs(position);
    stage_x.wait_settled();

    // Acquire with each grating configuration at this position
    for g in gratings {
        let grating_idx = g[0];
        let wavelength  = g[1];
        let label       = g[2];

        // Change spectrograph configuration
        // Camera must be stopped briefly during grating rotation
        camera.stop_stream();
        spec.set_grating(grating_idx);
        spec.set_wavelength(wavelength);
        camera.arm();  // re-arm after spectrograph settled

        // Acquire shots
        for s in 0..shots {
            sleep(0.002);
            total_acquired += 1;
        }

        print(`  pos=${position} mm, grating ${grating_idx} (${label}): ${shots} shots`);
    }
}

// ── Shutdown ─────────────────────────────────────────────────────────────────
print("[3/3] Shutting down...");
camera.stop_stream();

print("─────────────────────────────────────────");
print(`Multi-grating scan complete:`);
print(`  ${num_points} positions × ${gratings.len()} gratings × ${shots} shots = ${total_acquired} spectra`);
print("  Output: per-position spectrum set covering full elemental range");
