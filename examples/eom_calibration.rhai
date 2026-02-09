// EOM Calibration Script
//
// With rotators at max-transmission positions, sweeps EOM voltage (DAC0, 0-5V)
// at each wavelength while measuring power. Maps the EOM transfer function
// per wavelength.
//
// ⚠️  WARNING: DAC0 controls the EOM amplifier which affects laser power.
//    This script writes voltages to DAC0 intentionally for calibration.
//    Do NOT run this script unless you understand the EOM setup.
//
// Hardware:
//   - MaiTai Ti:Sapphire laser (shutter + wavelength control)
//   - Newport 1830-C power meter
//   - 3x ELL14 rotators on RS-485 bus (addresses 2, 3, 8)
//   - Comedi DAQ (NI PCI-MIO-16XE-10) — DAC0 for EOM voltage
//
// Prerequisites:
//   Run optic_identification.rhai first to determine rotator max-transmission
//   positions, then set the rotator_positions below.
//
// Output: HDF5 file with per-wavelength voltage-vs-power data
//
// Usage:
//   cargo run --release -p scripting --features scripting_full_comedi --bin rhai-runner -- examples/eom_calibration.rhai

print("╔════════════════════════════════════════════════════════╗");
print("║  EOM Calibration - Voltage Transfer Function          ║");
print("╚════════════════════════════════════════════════════════╝");
print("");

// =============================================================================
// Configuration
// =============================================================================

// Hardware ports (maitai machine, stable by-id paths)
const MAITAI_PORT = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const ELL14_PORT = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
const POWER_METER_PORT = "/dev/ttyS0";
const COMEDI_DEVICE = "/dev/comedi0";

// Rotator addresses and max-transmission positions (from optic_identification results)
// UPDATE THESE after running optic_identification.rhai!
let rotator_addrs = ["2", "3", "8"];
let rotator_positions = [0.0, 0.0, 0.0];  // degrees — set to max-transmission angles

// EOM sweep parameters
let eom_channel = 0;         // DAC0 = EOM amplifier
let voltage_start = 0.0;     // V
let voltage_end = 5.0;       // V
let voltage_step = 0.1;      // V
let wavelengths = [800.0, 900.0];  // nm — same as optic_identification
let pm_samples = 5;          // power meter averaging samples per reading
let settle_time_wl = 3.0;    // seconds to settle after wavelength change
let settle_time_eom = 0.1;   // seconds to settle after EOM voltage change

// Output file
let output_file = `eom_cal_${timestamp()}.h5`;

// Derived
let num_voltages = ((voltage_end - voltage_start) / voltage_step).to_int() + 1;
let total_points = wavelengths.len() * num_voltages;

print("Scan Parameters:");
print(`  Wavelengths:        ${wavelengths}`);
print(`  EOM voltage range:  ${voltage_start} V to ${voltage_end} V (step ${voltage_step} V)`);
print(`  Points/wavelength:  ${num_voltages}`);
print(`  PM averaging:       ${pm_samples} samples`);
print(`  Rotator positions:  ${rotator_positions}`);
print(`  Total points:       ${total_points}`);
print(`  Output file:        ${output_file}`);
print("");

// =============================================================================
// Initialize Hardware
// =============================================================================

print("Initializing hardware...");

let laser = create_maitai_tunable(MAITAI_PORT);
print("  ✓ MaiTai laser connected");

let pm = create_newport_1830c(POWER_METER_PORT);
print("  ✓ Newport 1830-C power meter connected");

let daq = create_comedi(COMEDI_DEVICE);
print(`  ✓ Comedi DAQ connected (${daq.board_name()})`);

let rotators = [];
for addr in rotator_addrs {
    let r = create_elliptec(ELL14_PORT, addr);
    print(`  ✓ ELL14 rotator ${addr} connected`);
    rotators.push(r);
}

// Zero EOM immediately on startup
daq.write_voltage(eom_channel, 0.0);
print("  ✓ EOM voltage zeroed (DAC0 = 0.0 V)");

let hdf5 = create_hdf5(output_file);
print(`  ✓ HDF5 file created: ${output_file}`);
print("");

// =============================================================================
// Write root metadata
// =============================================================================

hdf5.write_attr("script", "eom_calibration.rhai");
hdf5.write_attr("timestamp", timestamp_iso());
hdf5.write_attr("maitai_port", MAITAI_PORT);
hdf5.write_attr("ell14_port", ELL14_PORT);
hdf5.write_attr("power_meter_port", POWER_METER_PORT);
hdf5.write_attr("comedi_device", COMEDI_DEVICE);
hdf5.write_attr_i64("eom_channel", eom_channel);
hdf5.write_attr_i64("pm_samples", pm_samples);
hdf5.write_attr_f64("voltage_start", voltage_start);
hdf5.write_attr_f64("voltage_end", voltage_end);
hdf5.write_attr_f64("voltage_step", voltage_step);

// Store rotator positions as metadata
for i in 0..rotator_addrs.len() {
    hdf5.write_attr_f64(`rotator_${rotator_addrs[i]}_position_deg`, rotator_positions[i]);
}

// =============================================================================
// Position rotators at max-transmission angles
// =============================================================================

print("Positioning rotators at max-transmission angles...");
for i in 0..rotators.len() {
    rotators[i].move_abs(rotator_positions[i]);
    rotators[i].wait_settled();
    print(`  ✓ Rotator ${rotator_addrs[i]} → ${rotator_positions[i]}°`);
}
print("");

// =============================================================================
// Measurement Loop (shutter auto-closes on error)
// =============================================================================

print("Opening shutter and starting EOM sweep...");

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

            // Store actual rotator positions as group attributes
            for i in 0..rotator_addrs.len() {
                let actual_pos = rotators[i].position();
                hdf5.write_attr_f64(`${wl_group}_rotator_${rotator_addrs[i]}_deg`, actual_pos);
            }

            let data = [];

            let voltage = voltage_start;
            while voltage <= voltage_end + 0.001 {
                daq.write_voltage(eom_channel, voltage);
                sleep(settle_time_eom);

                let power = pm.read_averaged(pm_samples);
                data.push([voltage, power]);

                point_count += 1;
                if point_count % 10 == 0 {
                    print(`  [${point_count}/${total_points}] ${voltage} V → ${power} W`);
                }

                voltage += voltage_step;
            }

            // Write dataset: wavelength_800000pm/eom_sweep
            hdf5.write_array_2d(`${wl_group}/eom_sweep`, data);

            // Safety: zero EOM after each wavelength sweep
            daq.write_voltage(eom_channel, 0.0);
            print(`  ✓ ${wl} nm done (${data.len()} points saved, EOM zeroed)`);
        }
    } catch(err) {
        print(`⚠ Error during measurement: ${err}`);
        // Safety: always zero EOM on error to protect hardware
        daq.write_voltage(eom_channel, 0.0);
        hdf5.close();
        throw err;
    }
});

// =============================================================================
// Cleanup
// =============================================================================

print("");
print("Finalizing...");

// Safety: ensure EOM is zeroed
daq.write_voltage(eom_channel, 0.0);
print("  ✓ EOM voltage zeroed (DAC0 = 0.0 V)");

hdf5.close();
print(`  ✓ Data saved to ${output_file}`);
print("");
print(`EOM calibration complete! ${point_count} measurements recorded.`);
