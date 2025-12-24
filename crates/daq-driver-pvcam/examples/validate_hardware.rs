use anyhow::Result;
use daq_core::capabilities::Parameterized;
use daq_core::parameter::Parameter;
use daq_driver_pvcam::PvcamDriver;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("Initializing PVCAM Driver (Hardware Mode)...");

    // Attempt to connect to the first available camera
    // Note: User must run with --features pvcam_hardware for real interaction
    let driver = PvcamDriver::new_async("PrimeBSI".to_string()).await?;

    let params = driver.parameters();

    println!("--- Cmaera Metadata ---");

    if let Some(serial) = params.get_typed::<Parameter<String>>("info.serial_number") {
        println!("Serial Number: {}", serial.get());
    } else {
        println!("Serial Number: [MISSING]");
    }

    if let Some(fw) = params.get_typed::<Parameter<String>>("info.firmware_version") {
        println!("Firmware: {}", fw.get());
    } else {
        println!("Firmware: [MISSING]");
    }

    if let Some(model) = params.get_typed::<Parameter<String>>("info.model_name") {
        println!("Model: {}", model.get());
    } else {
        println!("Model: [MISSING]");
    }

    if let Some(depth) = params.get_typed::<Parameter<u16>>("info.bit_depth") {
        println!("Bit Depth: {}", depth.get());
    } else {
        println!("Bit Depth: [MISSING]");
    }

    // Verify it's not the mock
    if let Some(serial) = params.get_typed::<Parameter<String>>("info.serial_number") {
        if serial.get() == "MOCK-001" {
            println!("\nWARNING: Running in MOCK mode. Use --features pvcam_hardware to test real hardware.");
        } else {
            println!(
                "\nSUCCESS: Connected to real hardware (Serial: {}).",
                serial.get()
            );
        }
    }

    Ok(())
}
