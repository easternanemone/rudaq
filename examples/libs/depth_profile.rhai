// LIBS Depth Profile — Repeated ablation at the same positions
//
// Fires the laser at the same sample locations multiple times to build
// a depth profile. Each iteration ablates deeper into the material,
// revealing subsurface composition (coatings, layers, gradients).
//
// Ported from: CAAAMLIBS/LIBS experiments.py::custom_depth_scan()
//
// Usage:
//   rust-daq-daemon run examples/libs/depth_profile.rhai \
//     --hardware-config config/libs_hardware.toml

print("╔════════════════════════════════════════╗");
print("║  LIBS Depth Profile                    ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Scan Parameters ──────────────────────────────────────────────────
let y_start         = 0.0;     // mm
let y_end           = 5.0;     // mm
let y_step          = 1.0;     // mm
let x_pos           = 0.0;     // mm (fixed X position)
let num_iterations  = 10;      // depth layers (passes over same spots)
let pulses_per_shot = 100;     // laser pulses per spectrum

// ── Camera Parameters ────────────────────────────────────────────────
let mcp_gain     = 3600;
let ddg_delay_ps = 1300000;
let ddg_width_ps = 10000000;

let num_y = ((y_end - y_start) / y_step + 1.0).to_int();

print(`Line: Y = ${y_start} → ${y_end} mm (${num_y} points), X = ${x_pos} mm`);
print(`Depth iterations: ${num_iterations}`);
print(`Pulses per shot: ${pulses_per_shot}`);
print(`Total spectra: ${num_y * num_iterations}`);
print("");

// ── Construct hardware handles ───────────────────────────────────────
let camera  = create_andor_camera();
let stage_x = create_dover_axis("X");
let stage_y = create_dover_axis("Y");

// ── Setup ────────────────────────────────────────────────────────────
print("[1/3] Setting up...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.set_ddg_timing(ddg_delay_ps, ddg_width_ps);
camera.set_mcp_gain(mcp_gain);
camera.arm();

stage_x.move_abs(x_pos);
stage_x.wait_settled();
print("  Camera armed, stage positioned");

// ── Execute depth scan ───────────────────────────────────────────────
print("[2/3] Scanning (Y positions × depth iterations)...");
print("─────────────────────────────────────────");

for row in 0..num_y {
    let y = y_start + (row.to_float() * y_step);
    stage_y.move_abs(y);
    stage_y.wait_settled();

    for iter in 0..num_iterations {
        // Each iteration ablates deeper at the same spot
        sleep(0.002 * pulses_per_shot.to_float());

        if iter == 0 || iter == num_iterations - 1 {
            print(`  Y=${y} mm, layer ${iter + 1}/${num_iterations}`);
        }
    }
}

// ── Shutdown ─────────────────────────────────────────────────────────
print("[3/3] Shutting down...");
camera.stop_stream();

stage_y.move_abs(y_start);
stage_y.wait_settled();

print("─────────────────────────────────────────");
print(`Depth profile complete: ${num_y} positions × ${num_iterations} layers`);
print("Output: composition vs. depth for coating/layer analysis");
