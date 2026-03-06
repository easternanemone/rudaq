// LIBS Height Scan — Z-axis focus optimization
//
// Sweeps the Z axis (focus / standoff distance) while acquiring LIBS spectra
// to find the optimal focal position.  Maximum line intensity indicates
// the best focus for a given sample surface height.
//
// Usage:
//   rust-daq-daemon run examples/libs/height_scan.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Dover SmartStage Z axis connected (or mock mode)
//   - Andor iStar camera connected (or mock mode)

print("╔════════════════════════════════════════╗");
print("║  LIBS Height Scan (Focus Optimization) ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Height Scan Parameters ───────────────────────────────────────────────────
let z_start    = -2.0;   // mm relative to nominal focus
let z_end      =  2.0;   // mm
let z_step     =  0.2;   // mm step
let shots      = 5;      // shots per z position for averaging
let num_points = ((z_end - z_start) / z_step + 1.0).to_int();

// ── Camera Parameters ────────────────────────────────────────────────────────
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;

print(`Z scan: ${z_start} → ${z_end} mm in ${z_step} mm steps (${num_points} points)`);
print(`Shots per position: ${shots} (${num_points * shots} total)`);
print("");

// ── Construct hardware handles ────────────────────────────────────────────────
let camera  = create_andor_camera();
let stage_z = create_dover_axis("Z");

// ── Prepare Camera ───────────────────────────────────────────────────────────
print("[1/3] Preparing camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed");

// ── Execute Height Scan ──────────────────────────────────────────────────────
print("[2/3] Executing height scan...");
print("─────────────────────────────────────────");

for i in 0..num_points {
    let z_pos = z_start + (i.to_float() * z_step);

    stage_z.move_abs(z_pos);
    stage_z.wait_settled();

    // Acquire shots
    let shot_count = 0;
    for s in 0..shots {
        sleep(0.002);
        shot_count += 1;
    }

    print(`  [${i+1}/${num_points}] Z = ${z_pos} mm — ${shot_count} shots accumulated`);
}

// ── Shutdown ─────────────────────────────────────────────────────────────────
print("[3/3] Height scan complete, shutting down...");
camera.stop_stream();

print("─────────────────────────────────────────");
print(`Height scan complete: ${num_points} Z positions × ${shots} shots`);
print("Output: peak intensity vs. Z — maximum indicates optimal focus position");
