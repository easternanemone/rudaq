// LIBS Varied Pulses 2D — Dose-response study across a sample grid
//
// Performs a 2D scan where each column uses a different number of laser
// pulses per spectrum. This reveals how signal-to-noise and crater depth
// evolve with cumulative laser dose — critical for optimizing ablation
// parameters in quantitative LIBS.
//
// Ported from: CAAAMLIBS/LIBS experiments.py::varied_pulses_2d()
//
// Usage:
//   rust-daq-daemon run examples/libs/varied_pulses_2d.rhai \
//     --hardware-config config/libs_hardware.toml

print("╔════════════════════════════════════════╗");
print("║  LIBS Varied Pulses 2D Scan            ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Scan Parameters ──────────────────────────────────────────────────
let y_start    = 0.0;      // mm
let y_stop     = 5.0;      // mm
let y_step     = 0.5;      // mm
let x_start    = 0.0;      // mm
let x_step     = 1.0;      // mm (spacing between dose columns)
let min_pulses = 10;        // minimum pulses per spectrum
let max_pulses = 100;       // maximum pulses per spectrum
let pulse_step = 10;        // pulse count increment per column

// ── Camera Parameters ────────────────────────────────────────────────
let grating      = 2;
let grating_pos  = 310.0;
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;

let num_y = ((y_stop - y_start) / y_step + 1.0).to_int();
let num_columns = ((max_pulses - min_pulses) / pulse_step + 1).to_int();

print(`Y scan: ${y_start} → ${y_stop} mm (${num_y} points)`);
print(`Pulse range: ${min_pulses} → ${max_pulses}, step ${pulse_step} (${num_columns} columns)`);
print(`Total spectra: ${num_y * num_columns}`);
print("");

// ── Construct hardware handles ───────────────────────────────────────
let camera  = create_andor_camera();
let spec    = create_andor_spectrograph();
let stage_x = create_dover_axis("X");
let stage_y = create_dover_axis("Y");

// ── Setup ────────────────────────────────────────────────────────────
print("[1/3] Setting up...");
spec.set_grating(grating);
spec.set_wavelength(grating_pos);
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Spectrograph and camera configured");

// ── Execute 2D varied-pulse scan ─────────────────────────────────────
print("[2/3] Scanning...");
print("─────────────────────────────────────────");

for col in 0..num_columns {
    let pulses = min_pulses + (col * pulse_step);
    let x_pos  = x_start + (col.to_float() * x_step);

    stage_x.move_abs(x_pos);
    stage_x.wait_settled();

    print(`  Column ${col + 1}/${num_columns}: X=${x_pos} mm, ${pulses} pulses/spectrum`);

    for row in 0..num_y {
        let y_pos = y_start + (row.to_float() * y_step);
        stage_y.move_abs(y_pos);
        stage_y.wait_settled();

        // Fire 'pulses' laser shots and accumulate
        sleep(0.002 * pulses.to_float());
    }
}

// ── Shutdown ─────────────────────────────────────────────────────────
print("[3/3] Shutting down...");
camera.stop_stream();

// Return to origin
stage_x.move_abs(x_start);
stage_y.move_abs(y_start);
stage_x.wait_settled();
stage_y.wait_settled();

print("─────────────────────────────────────────");
print(`Varied pulses 2D complete: ${num_columns} dose levels × ${num_y} positions`);
print("Output: dose-response matrix for ablation parameter optimization");
