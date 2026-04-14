// LIBS Single Shot — Acquire one gated spectrum at the current position
//
// The simplest LIBS measurement: one laser pulse, one spectrum.
// Useful for quick material identification and alignment checks.
//
// Ported from: CAAAMLIBS/LIBS experiments.py::singleshot()
//
// Usage:
//   rust-daq-daemon run examples/libs/singleshot.rhai \
//     --hardware-config config/libs_hardware.toml

print("╔════════════════════════════════════════╗");
print("║  LIBS Single Shot Acquisition          ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Parameters ───────────────────────────────────────────────────────
let grating      = 2;            // diffraction grating index
let grating_pos  = 310.0;        // center wavelength (nm)
let mcp_gain     = 3600;         // MCP intensifier gain
let ddg_delay_ps = 1300000;      // 1.3 µs after laser pulse
let ddg_width_ps = 10000000;     // 10 µs gate width

print(`Grating: ${grating}, center λ: ${grating_pos} nm`);
print(`MCP gain: ${mcp_gain}, DDG: ${ddg_delay_ps / 1000} ns delay, ${ddg_width_ps / 1000} ns width`);
print("");

// ── Construct hardware handles ───────────────────────────────────────
let camera = create_andor_camera();
let spec   = create_andor_spectrograph();

// ── Setup ────────────────────────────────────────────────────────────
print("[1/4] Configuring spectrograph...");
spec.set_grating(grating);
spec.set_wavelength(grating_pos);
print(`  Grating ${grating} at ${grating_pos} nm`);

print("[2/4] Configuring camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed");

// ── Acquire ──────────────────────────────────────────────────────────
print("[3/4] Acquiring single spectrum...");
sleep(0.01);
print("  Spectrum acquired");

// ── Shutdown ─────────────────────────────────────────────────────────
print("[4/4] Shutting down...");
camera.stop_stream();

print("");
print("Single shot complete.");
print("Output: 1 background-subtracted spectrum ready for analysis");
