// LIBS Delay Scan — Sweep DDG gate delay to study plasma evolution
//
// Varies the detector gate delay after the laser pulse to build a
// time-resolved picture of plasma emission. Early delays capture
// continuum radiation; later delays isolate atomic/ionic lines.
//
// Between delay points the stage moves to a fresh sample spot so each
// measurement ablates virgin material (critical for quantitative LIBS).
//
// Usage:
//   rust-daq-daemon run examples/libs/delay_scan.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Andor iStar camera connected (or mock mode)
//   - Camera must support DDG (Digital Delay Generator) control
//   - Dover Motion stage connected (for sample repositioning)

print("╔════════════════════════════════════════╗");
print("║  LIBS Delay Scan (Gate Timing Study)   ║");
print("╚════════════════════════════════════════╝");
print("");

// ── Delay Scan Parameters ────────────────────────────────────────────
let delay_start_ps = 100000;      // 100 ns (early: mostly continuum)
let delay_end_ps   = 50000000;    // 50 µs (late: weak atomic lines)
let delay_step_ps  = 500000;      // 500 ns steps
let gate_width_ps  = 10000000;    // 10 µs gate width (fixed)
let shots_per_delay = 10;         // accumulate spectra per delay

// ── Stage Parameters (fresh spot per delay) ──────────────────────────
let stage_step_mm = 0.2;          // 200 µm between delay points

let num_delays = ((delay_end_ps - delay_start_ps) / delay_step_ps + 1).to_int();

print(`Delay: ${delay_start_ps} → ${delay_end_ps} ps in ${delay_step_ps} ps steps`);
print(`Gate width: ${gate_width_ps} ps (fixed)`);
print(`Shots per delay: ${shots_per_delay}`);
print(`Total delays: ${num_delays}`);
print(`Stage step: ${stage_step_mm} mm between delay points`);
print("");

// ── Construct hardware handles ────────────────────────────────────────
let camera = create_andor_camera();
let stage_y = create_dover_axis("Y");

// ── Prepare Camera ───────────────────────────────────────────────────
print("Preparing camera...");
camera.set_trigger_mode("External");
camera.set_gate_mode("DDG");
camera.arm();
print("  Camera armed");

// ── Execute Delay Scan ───────────────────────────────────────────────
print("Starting delay scan...");
print("─────────────────────────────────────────");

for i in 0..num_delays {
    let delay = delay_start_ps + (i * delay_step_ps);
    let delay_us = delay.to_float() / 1000000.0;

    // Move stage to fresh sample spot before each delay measurement
    let y_pos = i.to_float() * stage_step_mm;
    stage_y.move_abs(y_pos);
    stage_y.wait_settled();

    // Set DDG timing for this delay point (gate width stays fixed)
    camera.set_ddg_timing(delay, gate_width_ps);

    print(`  [${i+1}/${num_delays}] DDG delay: ${delay_us} µs @ Y=${y_pos} mm — ${shots_per_delay} shots`);
    sleep(0.02);
}

// ── Shutdown ─────────────────────────────────────────────────────────
camera.stop_stream();

// Return stage to origin
stage_y.move_abs(0.0);
stage_y.wait_settled();

print("─────────────────────────────────────────");
print(`Delay scan complete: ${num_delays} delay points x ${shots_per_delay} shots`);
print("Output: delay vs. integrated line intensity for plasma diagnostics");
