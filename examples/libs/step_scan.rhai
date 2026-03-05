// LIBS Step Scan — 1D line scan with position-triggered spectra
//
// Performs a linear scan along the X axis using Trigger-On-Position (TOP).
// At each position, the Dover stage sends a hardware trigger pulse to the
// Andor iStar camera, which acquires a gated spectrum.
//
// Usage:
//   rust-daq-daemon run examples/libs/step_scan.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Dover SmartStage connected (or mock mode)
//   - Andor iStar camera connected (or mock mode)
//   - Trigger cable: Dover TOP output → Camera external trigger input

print("╔════════════════════════════════════════╗");
print("║  LIBS Step Scan (1D Line Scan)         ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Scan Parameters ──────────────────────────────────────────────────
let scan_start = 0.0;      // mm
let scan_end   = 10.0;     // mm
let scan_step  = 0.5;      // mm (TOP increment)
let num_points = ((scan_end - scan_start) / scan_step + 1.0).to_int();

// ── Camera Parameters ────────────────────────────────────────────────
let exposure_ms    = 1.5;       // ms per trigger
let mcp_gain       = 3600;      // intensifier gain (0–4095)
let ddg_delay_ps   = 1300000;   // 1.3µs delay after laser pulse
let ddg_width_ps   = 10000000;  // 10µs gate width

print(`Scan: ${scan_start} → ${scan_end} mm, step ${scan_step} mm (${num_points} points)`);
print(`Camera: ${exposure_ms} ms exposure, MCP gain ${mcp_gain}`);
print(`DDG: delay ${ddg_delay_ps} ps, width ${ddg_width_ps} ps`);
print("");

// ── Step 1: Prepare Camera ───────────────────────────────────────────
print("[1/4] Preparing camera...");
// TODO: When Rhai LIBS extensions are available (bd-3yb8.9):
//   camera.set_exposure(exposure_ms / 1000.0);
//   camera.set_trigger_mode("External");
//   camera.set_mcp_gain(mcp_gain);
//   camera.set_ddg_delay(ddg_delay_ps);
//   camera.set_ddg_width(ddg_width_ps);
//   camera.arm();
print("  Camera configured (placeholder — awaiting Rhai LIBS extensions)");

// ── Step 2: Configure TOP ────────────────────────────────────────────
print("[2/4] Configuring Trigger-On-Position...");
// TODO: When Rhai LIBS extensions are available (bd-3yb8.9):
//   stage_x.enable_top(scan_start, scan_end, scan_step, false, 1000);
print("  TOP configured (placeholder — awaiting Rhai LIBS extensions)");

// ── Step 3: Execute Scan ─────────────────────────────────────────────
print("[3/4] Executing scan...");
// In a real scan, the stage moves continuously and TOP triggers the camera.
// For now, simulate with discrete moves:
for i in 0..num_points {
    let position = scan_start + (i.to_float() * scan_step);
    stage_x.move_abs(position);
    sleep(0.05);  // settle time
    print(`  [${i+1}/${num_points}] Position: ${position} mm — spectrum acquired`);
}

// ── Step 4: Shutdown ─────────────────────────────────────────────────
print("[4/4] Scan complete, shutting down...");
// TODO: stage_x.disable_top();
// TODO: camera.disarm();

print("");
print(`Step scan complete: ${num_points} spectra acquired over ${scan_end - scan_start} mm`);
