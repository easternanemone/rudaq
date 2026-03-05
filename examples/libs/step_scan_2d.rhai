// LIBS 2D Mapping Scan — Raster scan with spectra at each grid point
//
// Performs a 2D raster scan over X and Y axes. At each grid point
// the camera acquires a gated spectrum. Serpentine (boustrophedon)
// row ordering minimizes total stage travel.
//
// Usage:
//   rust-daq-daemon run examples/libs/step_scan_2d.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Dover SmartStage X and Y axes connected (or mock mode)
//   - Andor iStar camera connected (or mock mode)
//   - Trigger cable: Dover TOP output → Camera external trigger input

print("╔════════════════════════════════════════╗");
print("║  LIBS 2D Mapping Scan                  ║");
print("╚════════════════════════════════════════╝");
print("");

// ── 2D Scan Parameters ──────────────────────────────────────────────
let x_start = 0.0;     // mm
let x_end   = 5.0;     // mm
let x_step  = 0.5;     // mm

let y_start = 0.0;     // mm
let y_end   = 5.0;     // mm
let y_step  = 0.5;     // mm

let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;

let nx = ((x_end - x_start) / x_step + 1.0).to_int();
let ny = ((y_end - y_start) / y_step + 1.0).to_int();
let total = nx * ny;

print(`X: ${x_start} → ${x_end} mm, step ${x_step} mm (${nx} points)`);
print(`Y: ${y_start} → ${y_end} mm, step ${y_step} mm (${ny} points)`);
print(`Total grid points: ${total}`);
print("");

// ── Construct hardware handles ────────────────────────────────────────
let camera  = create_andor_camera();
let stage_x = create_dover_axis("X");
let stage_y = create_dover_axis("Y");

// ── Prepare Camera ───────────────────────────────────────────────────
print("Preparing camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();
print("  Camera armed");

// ── Execute 2D Raster Scan ──────────────────────────────────────────
print("Starting 2D raster scan...");
print("─────────────────────────────────────────");

let count = 0;
for iy in 0..ny {
    let y_pos = y_start + (iy.to_float() * y_step);

    // Move Y axis to row position
    stage_y.move_abs(y_pos);
    stage_y.wait_settled();

    // Scan X axis across the row (serpentine: alternate direction each row)
    let x_forward = iy % 2 == 0;

    for ix in 0..nx {
        let x_idx = if x_forward { ix } else { nx - 1 - ix };
        let x_pos = x_start + (x_idx.to_float() * x_step);

        stage_x.move_abs(x_pos);
        stage_x.wait_settled();

        count += 1;
        if count % 10 == 0 || count == total {
            print(`  [${count}/${total}] (${x_pos}, ${y_pos}) mm`);
        }
    }
}

// ── Shutdown ─────────────────────────────────────────────────────────
camera.stop_stream();

print("─────────────────────────────────────────");
print(`2D scan complete: ${count} spectra in ${nx}x${ny} grid`);
print("Output: elemental composition map (X,Y) → spectrum");
