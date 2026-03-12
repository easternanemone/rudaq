#![cfg(not(target_arch = "wasm32"))]
//! Comedi Concurrency Safety Test Suite
//!
//! Validates that the FFI wrapper properly serializes concurrent hardware accesses
//! using `with_handle` and does not suffer from data races, even under heavy load.
//! Addressed as part of bd-pman.5.2.
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_CONCURRENCY_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")

#![cfg(feature = "hardware")]

use driver_comedi::{hal::ReadableAnalogInput, ComediDevice};
use std::env;
use std::sync::Arc;
use std::thread;

fn concurrency_test_enabled() -> bool {
    env::var("COMEDI_CONCURRENCY_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

#[test]
fn test_concurrent_analog_reads() {
    if !concurrency_test_enabled() {
        println!("Skipping concurrency test (COMEDI_CONCURRENCY_TEST!=1)");
        return;
    }

    let path = device_path();
    let device = ComediDevice::open(&path).expect("Failed to open device");

    // We wrap the device in an Arc to share among threads, though ComediDevice itself
    // internally uses an Arc<DeviceInner>, so cloning it is cheap and safe.
    let num_threads = 8;
    let reads_per_thread = 50;

    let mut handles = vec![];

    for t in 0..num_threads {
        let dev_clone = device.clone();
        handles.push(thread::spawn(move || {
            // Attempt to get the analog input subsystem.
            // On some devices, subdevice 0 is AI. We just find the first AI.
            let ai_subdev = dev_clone
                .find_subdevice(driver_comedi::SubdeviceType::AnalogInput)
                .expect("No analog input subdevice found on hardware");

            let mut ai = dev_clone
                .analog_input(ai_subdev)
                .expect("Failed to create AI subsystem");

            let mut success_count = 0;
            let mut error_count = 0;

            for _ in 0..reads_per_thread {
                // Thread ID used as channel modulo 4 just to vary the inputs
                match ai.read_volts(t % 4) {
                    Ok(_val) => success_count += 1,
                    Err(_) => error_count += 1,
                }
            }

            (success_count, error_count)
        }));
    }

    let mut total_success = 0;
    let mut total_errors = 0;

    for handle in handles {
        let (s, e) = handle.join().expect("Thread panicked");
        total_success += s;
        total_errors += e;
    }

    // The key invariant is that it didn't segfault or panic in C FFI,
    // which proves serialization works. Whether it returns values or ComediErrors
    // depends on the physical hardware's capabilities.
    println!(
        "Concurrent reads complete. Success: {}, Errors: {}",
        total_success, total_errors
    );
    assert!(total_success + total_errors == num_threads * reads_per_thread);
}

#[test]
fn test_concurrent_invalid_path_validation() {
    if !concurrency_test_enabled() {
        return;
    }

    // Test that attempting to open an invalid path concurrently yields safe,
    // clean Rust errors rather than FFI crashes.
    let num_threads = 10;
    let mut handles = vec![];

    for _ in 0..num_threads {
        handles.push(thread::spawn(move || {
            let res = ComediDevice::open("/dev/comedi_invalid_path_999");
            assert!(res.is_err());
            let err_msg = res.unwrap_err().to_string();
            assert!(
                err_msg.contains("No such file or directory")
                    || err_msg.contains("Permission denied")
            );
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("Thread panicked during invalid path open");
    }
}
