// LIBS Step Scan — 1D line scan with position-triggered spectra
//
// Performs a linear scan along the X axis. At each position the stage
// moves to the target location and acquires a gated spectrum via the
// Andor iStar camera (or mock equivalents in development mode).
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
let scan_step  = 0.5;      // mm
let num_points = ((scan_end - scan_start) / scan_step + 1.0).to_int();

// ── Camera Parameters ────────────────────────────────────────────────
let mcp_gain     = 3600;      // intensifier gain (0–4095)
let ddg_delay_ps = 1300000;   // 1.3 µs delay after laser pulse
let ddg_width_ps = 10000000;  // 10 µs gate width

print(`Scan: ${scan_start} → ${scan_end} mm, step ${scan_step} mm (${num_points} points)`);
print(`Camera: MCP gain ${mcp_gain}, DDG delay ${ddg_delay_ps} ps, width ${ddg_width_ps} ps`);
print("");

// ── Construct hardware handles ────────────────────────────────────────
let camera  = create_andor_camera();
let stage_x = create_dover_axis("X");

// ── Step 1: Prepare Camera ───────────────────────────────────────────
print("[1/4] Preparing camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed");

// ── Step 2: Configure TOP ────────────────────────────────────────────
print("[2/4] Configuring Trigger-On-Position...");
stage_x.enable_top(scan_start, scan_end, scan_step, false, 1000);
print(`  TOP: ${scan_start}→${scan_end} mm, Δ${scan_step} mm, 1 µs pulse`);

// ── Step 3: Execute Scan ─────────────────────────────────────────────
print("[3/4] Executing scan...");
for i in 0..num_points {
    let position = scan_start + (i.to_float() * scan_step);
    stage_x.move_abs(position);
    stage_x.wait_settled();
    print(`  [${i+1}/${num_points}] Position: ${position} mm — spectrum acquired`);
}

// ── Step 4: Shutdown ─────────────────────────────────────────────────
print("[4/4] Scan complete, shutting down...");
stage_x.disable_top();
camera.stop_stream();

print("");
print(`Step scan complete: ${num_points} spectra acquired over ${scan_end - scan_start} mm`);
