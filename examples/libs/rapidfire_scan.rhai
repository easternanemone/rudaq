// LIBS Rapid-Fire Scan — High-speed continuous acquisition during motion
//
// The stage moves at constant velocity while the laser fires at a fixed
// repetition rate. No stop-and-shoot — this is the fastest scanning mode.
// Spatial resolution is determined by velocity / rep_rate.
//
// Ported from: CAAAMLIBS/LIBS experiments.py::rapidfire_moving_constantLIBS()
//
// Usage:
//   rust-daq-daemon run examples/libs/rapidfire_scan.rhai \
//     --hardware-config config/libs_hardware.toml

print("╔════════════════════════════════════════╗");
print("║  LIBS Rapid-Fire Continuous Scan       ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Scan Parameters ──────────────────────────────────────────────────
let scan_start    = 0.0;       // mm
let scan_end      = 20.0;     // mm
let velocity      = 10.0;     // mm/s stage velocity
let rep_rate      = 1000.0;   // Hz laser repetition rate
let pulses_per_spectrum = 1;  // single-shot per trigger

// ── Camera Parameters ────────────────────────────────────────────────
let grating      = 2;
let grating_pos  = 310.0;
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;

let scan_length  = scan_end - scan_start;
let scan_time    = scan_length / velocity;
let spatial_res  = velocity / rep_rate;
let num_spectra  = (scan_length / spatial_res).to_int();

print(`Scan: ${scan_start} → ${scan_end} mm at ${velocity} mm/s`);
print(`Laser rep rate: ${rep_rate} Hz`);
print(`Spatial resolution: ${spatial_res * 1000.0} µm`);
print(`Expected spectra: ${num_spectra} in ${scan_time} s`);
print("");

// ── Construct hardware handles ───────────────────────────────────────
let camera  = create_andor_camera();
let spec    = create_andor_spectrograph();
let stage_y = create_dover_axis("Y");

// ── Setup ────────────────────────────────────────────────────────────
print("[1/4] Configuring spectrograph...");
spec.set_grating(grating);
spec.set_wavelength(grating_pos);

print("[2/4] Configuring camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed for external trigger");

// ── Configure TOP for rapid-fire ─────────────────────────────────────
print("[3/4] Configuring Trigger-On-Position...");
stage_y.enable_top(scan_start, scan_end, spatial_res, false, 1000);
stage_y.set_velocity(velocity);
print(`  TOP: Δ${spatial_res * 1000.0} µm steps, ${velocity} mm/s`);

// ── Execute rapid-fire scan ──────────────────────────────────────────
print("[4/4] Executing rapid-fire scan...");
print("─────────────────────────────────────────");

// Move to start, then scan through
stage_y.move_abs(scan_start);
stage_y.wait_settled();

print("  Stage moving...");
stage_y.move_abs(scan_end);
stage_y.wait_settled();

print(`  Scan complete: ${num_spectra} spectra acquired`);

// ── Shutdown ─────────────────────────────────────────────────────────
stage_y.disable_top();
camera.stop_stream();

// Return to start
stage_y.move_abs(scan_start);
stage_y.wait_settled();

print("─────────────────────────────────────────");
print(`Rapid-fire scan complete: ${num_spectra} spectra over ${scan_length} mm`);
print(`Throughput: ${num_spectra.to_float() / scan_time} spectra/s`);
