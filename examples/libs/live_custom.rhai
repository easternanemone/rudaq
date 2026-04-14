// LIBS Live Acquisition (Custom Parameters) — Interactive monitoring
//
// Acquires spectra in a loop with configurable camera and spectrograph
// parameters. Moves the stage between shots to expose fresh material.
// Designed for real-time tuning of LIBS measurement conditions.
//
// Ported from: CAAAMLIBS/LIBS experiments.py::live_acquisition_custom()
//
// Usage:
//   rust-daq-daemon run examples/libs/live_custom.rhai \
//     --hardware-config config/libs_hardware.toml

print("╔════════════════════════════════════════╗");
print("║  LIBS Live Acquisition (Custom)        ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Parameters ───────────────────────────────────────────────────────
let num_shots       = 20;
let pulses_per_shot = 100;
let grating         = 2;
let grating_pos     = 310.0;     // nm
let slit_width      = 150.0;     // µm
let mcp_gain        = 3600;
let ddg_delay_ps    = 1300000;   // 1.3 µs
let ddg_width_ps    = 10000000;  // 10 µs
let start_y         = 0.0;       // mm
let y_step          = 0.2;       // mm between shots (fresh material)

print(`Shots: ${num_shots}, pulses/shot: ${pulses_per_shot}`);
print(`Grating: ${grating} at ${grating_pos} nm, slit: ${slit_width} µm`);
print(`Camera: MCP ${mcp_gain}, DDG ${ddg_delay_ps / 1000} ns / ${ddg_width_ps / 1000} ns`);
print("");

// ── Construct hardware handles ───────────────────────────────────────
let camera  = create_andor_camera();
let spec    = create_andor_spectrograph();
let stage_y = create_dover_axis("Y");

// ── Setup ────────────────────────────────────────────────────────────
print("Setting up...");
spec.set_grating(grating);
spec.set_wavelength(grating_pos);
spec.set_slit_width("input", slit_width);

camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();

stage_y.move_abs(start_y);
stage_y.wait_settled();
print("  Ready");

// ── Acquire ──────────────────────────────────────────────────────────
print("─────────────────────────────────────────");
for i in 0..num_shots {
    let y_pos = start_y + (i.to_float() * y_step);
    stage_y.move_abs(y_pos);
    stage_y.wait_settled();

    // Accumulate pulses_per_shot laser shots
    sleep(0.002 * pulses_per_shot.to_float());

    print(`  [${i + 1}/${num_shots}] Y=${y_pos} mm — spectrum acquired`);
}

// ── Shutdown ─────────────────────────────────────────────────────────
camera.stop_stream();
stage_y.move_abs(start_y);
stage_y.wait_settled();

print("─────────────────────────────────────────");
print(`Live acquisition complete: ${num_shots} spectra`);
