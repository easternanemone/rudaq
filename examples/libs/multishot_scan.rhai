// LIBS Multishot Step Scan — Accumulate multiple shots per position
//
// At each scan position, fires N laser shots and accumulates the spectra
// before moving to the next point.  Multi-shot accumulation improves
// signal-to-noise for trace element detection.
//
// Usage:
//   rust-daq-daemon run examples/libs/multishot_scan.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Dover SmartStage X axis connected (or mock mode)
//   - Andor iStar camera in external trigger mode

print("╔════════════════════════════════════════╗");
print("║  LIBS Multishot Step Scan              ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Scan Parameters ──────────────────────────────────────────────────────────
let scan_start  = 0.0;     // mm
let scan_end    = 10.0;    // mm
let scan_step   = 0.5;     // mm
let shots       = 5;       // laser shots to accumulate per position
let num_points  = ((scan_end - scan_start) / scan_step + 1.0).to_int();

// ── Camera Parameters ────────────────────────────────────────────────────────
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;   // 1.3 µs
let ddg_width_ps = 10000000;  // 10 µs

print(`Scan: ${scan_start} → ${scan_end} mm, step ${scan_step} mm (${num_points} points)`);
print(`Shots per position: ${shots} (${num_points * shots} total acquisitions)`);
print(`MCP gain: ${mcp_gain}, DDG: ${ddg_delay_ps / 1000} ns delay`);
print("");

// ── Construct hardware handles ────────────────────────────────────────────────
let camera  = create_andor_camera();
let stage_x = create_dover_axis("X");

// ── Prepare Camera ───────────────────────────────────────────────────────────
print("[1/3] Preparing camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed");

// ── Execute Multishot Scan ───────────────────────────────────────────────────
print("[2/3] Executing multishot scan...");
print("─────────────────────────────────────────");

let count = 0;
for i in 0..num_points {
    let position = scan_start + (i.to_float() * scan_step);

    stage_x.move_abs(position);
    stage_x.wait_settled();

    // Accumulate N shots at this position
    for s in 0..shots {
        sleep(0.002);  // ~2 ms per shot at 500 Hz laser rep rate
        count += 1;
    }

    if i % 5 == 0 || i == num_points - 1 {
        let pct = ((i + 1).to_float() / num_points.to_float() * 100.0).to_int();
        print(`  [${i+1}/${num_points}] pos=${position} mm — ${shots} shots acquired (${pct}%)`);
    }
}

// ── Shutdown ─────────────────────────────────────────────────────────────────
print("[3/3] Scan complete, shutting down...");
camera.stop_stream();

print("─────────────────────────────────────────");
print(`Multishot scan complete: ${num_points} positions × ${shots} shots = ${count} total`);
print(`Effective noise reduction: √${shots} ≈ ${(shots.to_float().sqrt() * 10.0).to_int().to_float() / 10.0}×`);
