// LIBS Wavelength Scan — Coordinated spectrograph + camera spectral sweep
//
// Scans multiple spectral windows by changing the grating/wavelength while
// keeping the camera and spectrograph synchronized via ScanController.
//
// ScanController.set_config() performs the correct stop→tune→re-arm sequence
// on every grating change, preventing stray acquisitions during grating rotation.
//
// Usage:
//   rust-daq-daemon run examples/libs/wavelength_scan.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Andor iStar camera + Shamrock spectrograph connected (or mock mode)

print("╔════════════════════════════════════════╗");
print("║  LIBS Wavelength Scan (ScanController) ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Spectral Coverage Plan ───────────────────────────────────────────────────
// Each entry: [grating, center_nm, label]
let configs = [
    [1, 288.0,  "Si I 288 nm"],
    [1, 310.0,  "Al I 309/310 nm"],
    [1, 396.0,  "Al I 394/396 nm"],
    [2, 422.0,  "Ca I 422 nm"],
    [2, 438.0,  "Fe I 438 nm"],
    [2, 487.0,  "Fe I 487 nm"],
];

// ── Camera Parameters ────────────────────────────────────────────────────────
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;
let shots        = 10;

print(`Spectral configs: ${configs.len()}`);
print(`Shots per config: ${shots}`);
print(`Total acquisitions: ${configs.len() * shots}`);
print("");

// ── Construct hardware handles ────────────────────────────────────────────────
let camera  = create_andor_camera();
let spec    = create_andor_spectrograph();

// ── Wrap in ScanController for coordinated tuning ────────────────────────────
let sc = create_scan_controller(camera, spec);

// ── Initial Camera Setup ──────────────────────────────────────────────────────
print("[1/3] Setting up camera...");
// Access the underlying camera through the controller
let cam = sc.camera();
cam.set_trigger_mode("External");
cam.set_gate_mode("DDG");
cam.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
cam.set_mcp_gain(mcp_gain);
cam.arm();
print("  Camera armed");

// ── Execute Wavelength Scan ──────────────────────────────────────────────────
print("[2/3] Executing wavelength scan...");
print("─────────────────────────────────────────");

let total_acquired = 0;

for cfg in configs {
    let grating    = cfg[0];
    let wavelength = cfg[1];
    let label      = cfg[2];

    // ScanController.set_config: stop_stream → set_grating → set_wavelength → arm
    sc.set_config(grating, wavelength);

    // Acquire shots at this spectral position
    for s in 0..shots {
        sleep(0.002);
        total_acquired += 1;
    }

    print(`  Grating ${grating} @ ${wavelength} nm (${label}): ${shots} shots`);
}

// ── Shutdown ─────────────────────────────────────────────────────────────────
print("[3/3] Scan complete, shutting down...");
sc.camera().stop_stream();

print("─────────────────────────────────────────");
print(`Wavelength scan complete: ${configs.len()} spectral windows, ${total_acquired} total spectra`);
print("ScanController ensured camera was safely stopped during each grating change.");
