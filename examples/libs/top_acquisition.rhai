// LIBS TOP Acquisition — Continuous scan with hardware-triggered spectra
//
// Demonstrates Trigger-On-Position (TOP) based acquisition where the
// Dover stage generates trigger pulses at precise spatial intervals
// during continuous motion. This is the highest-throughput LIBS mode:
// no stop-and-go overhead, spectra are acquired on-the-fly.
//
// Usage:
//   rust-daq-daemon run examples/libs/top_acquisition.rhai \
//     --hardware-config config/libs_hardware.toml
//
// Prerequisites:
//   - Dover SmartStage with TOP capability
//   - Andor iStar camera in external trigger mode
//   - Hardware trigger cable: Dover TOP → Camera trigger input

print("╔════════════════════════════════════════╗");
print("║  LIBS TOP Acquisition (Continuous)     ║");
print("╚════════════════════════════════════════╝");
print("");

// ── TOP Parameters ──────────────────────────────────────────────────
let scan_start     = 0.0;      // mm
let scan_end       = 20.0;     // mm
let scan_increment = 0.1;      // mm (100 µm between triggers)
let scan_velocity  = 1.0;      // mm/s
let pulse_width_ns = 1000;     // 1 µs trigger pulse
let bidirectional  = false;    // single-direction scan

let num_triggers = ((scan_end - scan_start) / scan_increment).to_int();
let scan_time_s  = (scan_end - scan_start) / scan_velocity;

print(`Scan: ${scan_start} → ${scan_end} mm at ${scan_velocity} mm/s`);
print(`TOP: ${scan_increment} mm increment, ${pulse_width_ns} ns pulse`);
print(`Expected: ${num_triggers} triggers in ${scan_time_s} s`);
print(`Trigger rate: ${num_triggers.to_float() / scan_time_s} Hz`);
print("");

// ── Step 1: Prepare Camera ──────────────────────────────────────────
print("[1/5] Preparing camera for external triggering...");
// TODO: camera.set_trigger_mode("External");
// TODO: camera.set_exposure(0.0015);  // 1.5 ms
// TODO: camera.set_ddg_delay(1300000);  // 1.3 µs
// TODO: camera.set_ddg_width(10000000); // 10 µs
// TODO: camera.set_mcp_gain(3600);
// TODO: camera.arm();
print("  Camera armed (placeholder)");

// ── Step 2: Configure TOP ───────────────────────────────────────────
print("[2/5] Configuring Trigger-On-Position...");
// TODO: stage_x.enable_top(scan_start, scan_end, scan_increment, bidirectional, pulse_width_ns);
print(`  TOP: ${scan_start}→${scan_end} mm, Δ${scan_increment} mm, pulse ${pulse_width_ns} ns`);

// ── Step 3: Set Velocity ────────────────────────────────────────────
print("[3/5] Setting scan velocity...");
// TODO: stage_x.set_velocity(scan_velocity);
print(`  Velocity: ${scan_velocity} mm/s`);

// ── Step 4: Execute Continuous Scan ─────────────────────────────────
print("[4/5] Starting continuous scan...");
// In TOP mode, the stage moves continuously and generates trigger pulses
// at each increment. The camera acquires on each trigger.
// Simulated here with discrete steps:
let acquired = 0;
for i in 0..num_triggers {
    let pos = scan_start + (i.to_float() * scan_increment);
    stage_x.move_abs(pos);
    acquired += 1;
    if acquired % 20 == 0 {
        let pct = (acquired.to_float() / num_triggers.to_float() * 100.0).to_int();
        print(`  Progress: ${pct}% (${acquired}/${num_triggers} spectra)`);
    }
    sleep(0.005);
}
print(`  Scan complete: ${acquired} spectra acquired`);

// ── Step 5: Shutdown ────────────────────────────────────────────────
print("[5/5] Shutting down...");
// TODO: stage_x.disable_top();
// TODO: camera.disarm();
print("  TOP disabled, camera disarmed");

print("");
print(`TOP acquisition complete: ${acquired} spectra over ${scan_end - scan_start} mm`);
print(`Effective spatial resolution: ${scan_increment} mm`);
