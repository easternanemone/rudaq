// LIBS Delay Scan — Sweep DDG gate delay to study plasma evolution
//
// Varies the detector gate delay after the laser pulse to build a
// time-resolved picture of plasma emission. Early delays capture
// continuum radiation; later delays isolate atomic/ionic lines.
//
// Usage:
//   rust-daq-daemon run examples/libs/delay_scan.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Andor iStar camera connected (or mock mode)
//   - Camera must support DDG (Digital Delay Generator) control

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

let num_delays = ((delay_end_ps - delay_start_ps) / delay_step_ps + 1).to_int();

print(`Delay: ${delay_start_ps} → ${delay_end_ps} ps in ${delay_step_ps} ps steps`);
print(`Gate width: ${gate_width_ps} ps (fixed)`);
print(`Shots per delay: ${shots_per_delay}`);
print(`Total delays: ${num_delays}`);
print("");

// ── Execute Delay Scan ───────────────────────────────────────────────
print("Starting delay scan...");
print("─────────────────────────────────────────");

for i in 0..num_delays {
    let delay = delay_start_ps + (i * delay_step_ps);
    let delay_us = delay.to_float() / 1000000.0;

    // TODO: When Rhai LIBS extensions are available (bd-3yb8.9):
    //   camera.set_ddg_delay(delay);
    //   for _shot in 0..shots_per_delay {
    //       camera.trigger();
    //       camera.wait_frame();
    //   }

    print(`  [${i+1}/${num_delays}] DDG delay: ${delay_us} µs — ${shots_per_delay} shots accumulated`);
    sleep(0.02);
}

print("─────────────────────────────────────────");
print(`Delay scan complete: ${num_delays} delay points x ${shots_per_delay} shots`);
print("Output: delay vs. integrated line intensity for plasma diagnostics");
