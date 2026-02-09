// Optic Identification Script
//
// Sweeps all 3 ELL14 rotators through 180 degrees while measuring power at
// each angle and wavelength. Identifies which rotator holds which optic
// (half waveplate, quarter waveplate, linear polarizer) by their transmission
// profiles.
//
// Hardware:
//   - MaiTai Ti:Sapphire laser (shutter + wavelength control)
//   - Newport 1830-C power meter
//   - 3x ELL14 rotators on RS-485 bus (addresses 2, 3, 8)
//
// Output: HDF5 file with per-wavelength, per-rotator angle-vs-power data
//
// Usage:
//   cargo run --release -p scripting --features scripting_full --bin rhai-runner -- examples/optic_identification.rhai

print("╔════════════════════════════════════════════════════════╗");
print("║  Optic Identification - Rotator Transmission Profiles ║");
print("╚════════════════════════════════════════════════════════╝");
print("");

// =============================================================================
// Configuration
// =============================================================================

// Hardware ports (maitai machine, stable by-id paths)
const MAITAI_PORT = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const ELL14_PORT = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
const POWER_METER_PORT = "/dev/ttyS0";

// Rotator addresses on RS-485 bus
let rotator_addrs = ["2", "3", "8"];

// Scan parameters
let angle_start = 0.0;       // degrees
let angle_end = 180.0;       // degrees
let angle_step = 10.0;       // degrees
let wavelengths = [800.0, 900.0];  // nm — add more as needed
let pm_samples = 5;          // power meter averaging samples per reading
let settle_time_wl = 3.0;    // seconds to settle after wavelength change
let settle_time_move = 0.1;  // seconds to settle after rotator move

// Output file
let output_file = `optic_id_${timestamp()}.h5`;

// Derived
let num_angles = ((angle_end - angle_start) / angle_step).to_int() + 1;
let total_points = wavelengths.len() * rotator_addrs.len() * num_angles;

print("Scan Parameters:");
print(`  Wavelengths:     ${wavelengths}`);
print(`  Angle range:     ${angle_start}° to ${angle_end}° (step ${angle_step}°)`);
print(`  Points/rotator:  ${num_angles}`);
print(`  Rotators:        ${rotator_addrs}`);
print(`  PM averaging:    ${pm_samples} samples`);
print(`  Total points:    ${total_points}`);
print(`  Output file:     ${output_file}`);
print("");

// =============================================================================
// Initialize Hardware
// =============================================================================

print("Initializing hardware...");

let laser = create_maitai_tunable(MAITAI_PORT);
print("  ✓ MaiTai laser connected");

let pm = create_newport_1830c(POWER_METER_PORT);
print("  ✓ Newport 1830-C power meter connected");

let rotators = [];
for addr in rotator_addrs {
    let r = create_elliptec(ELL14_PORT, addr);
    print(`  ✓ ELL14 rotator ${addr} connected (velocity: ${r.velocity()}%)`);
    rotators.push(r);
}

let hdf5 = create_hdf5(output_file);
print(`  ✓ HDF5 file created: ${output_file}`);
print("");

// =============================================================================
// Write root metadata
// =============================================================================

hdf5.write_attr("script", "optic_identification.rhai");
hdf5.write_attr("timestamp", timestamp_iso());
hdf5.write_attr("maitai_port", MAITAI_PORT);
hdf5.write_attr("ell14_port", ELL14_PORT);
hdf5.write_attr("power_meter_port", POWER_METER_PORT);
hdf5.write_attr_i64("pm_samples", pm_samples);
hdf5.write_attr_f64("angle_start_deg", angle_start);
hdf5.write_attr_f64("angle_end_deg", angle_end);
hdf5.write_attr_f64("angle_step_deg", angle_step);

// =============================================================================
// Measurement Loop (shutter auto-closes on error)
// =============================================================================

print("Opening shutter and starting measurements...");

let point_count = 0;

with_shutter_open(laser.as_shutter(), || {
    try {
        for wl in wavelengths {
            print("");
            print(`━━━ Wavelength: ${wl} nm ━━━`);

            laser.set_wavelength(wl);
            pm.set_wavelength(wl);
            sleep(settle_time_wl);

            // Use picometers in group name to avoid truncation collisions
            let wl_pm = (wl * 1000.0).round().to_int();
            let wl_group = `wavelength_${wl_pm}pm`;
            hdf5.create_group(wl_group);
            hdf5.write_attr_f64(`${wl_group}_wavelength_nm`, wl);

            for i in 0..rotator_addrs.len() {
                let addr = rotator_addrs[i];
                let rotator = rotators[i];

                print(`  Rotator ${addr}: sweeping ${angle_start}° to ${angle_end}°`);

                let data = [];

                let angle = angle_start;
                while angle <= angle_end + 0.001 {
                    rotator.move_abs(angle);
                    rotator.wait_settled();
                    sleep(settle_time_move);

                    let power = pm.read_averaged(pm_samples);
                    data.push([angle, power]);

                    point_count += 1;
                    if point_count % 10 == 0 {
                        print(`    [${point_count}/${total_points}] ${angle}° → ${power} W`);
                    }

                    angle += angle_step;
                }

                // Write dataset: wavelength_800000pm/rotator_2
                let dataset_path = `${wl_group}/rotator_${addr}`;
                hdf5.write_array_2d(dataset_path, data);

                print(`    ✓ Rotator ${addr} done (${data.len()} points saved)`);
            }
        }
    } catch(err) {
        print(`⚠ Error during measurement: ${err}`);
        // Ensure cleanup happens even on error
        for i in 0..rotators.len() {
            rotators[i].home();
        }
        hdf5.close();
        throw err;
    }
});

// =============================================================================
// Cleanup
// =============================================================================

print("");
print("Finalizing...");

// Home all rotators
for i in 0..rotators.len() {
    rotators[i].home();
    print(`  ✓ Rotator ${rotator_addrs[i]} homed`);
}

hdf5.close();
print(`  ✓ Data saved to ${output_file}`);
print("");
print(`Optic identification complete! ${point_count} measurements recorded.`);
