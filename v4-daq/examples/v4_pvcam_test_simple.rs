//! V4 PVCAM Camera Test (Simplified)

use kameo::Actor;
use v4_daq::actors::pvcam::{
    ConfigureROI, GetCapabilities, PVCAMActor, SetBinning, SetGain, SetTiming, SnapFrame,
    StartStream, StopStream,
};
use v4_daq::traits::camera_sensor::{
    BinningConfig, CameraTiming, RegionOfInterest, TriggerMode,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("=== V4 PVCAM Camera Test ===\n");

    // Create mock PVCAM actor
    let actor = PVCAMActor::spawn(PVCAMActor::mock(
        "test_pvcam".to_string(),
        "PrimeBSI".to_string(),
    ));

    // Test 1: Get capabilities
    println!("Test 1: Get camera capabilities");
    let caps = actor
        .ask(GetCapabilities)
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!(
        "  Sensor: {}x{}, Formats: {:?}",
        caps.sensor_width, caps.sensor_height, caps.pixel_formats
    );
    println!(
        "  Exposure: {}µs - {}µs",
        caps.min_exposure_us, caps.max_exposure_us
    );
    println!("  Max binning: {}x{}\n", caps.max_binning_x, caps.max_binning_y);

    // Test 2: Configure ROI
    println!("Test 2: Configure ROI to 512x512");
    actor
        .ask(ConfigureROI {
            roi: RegionOfInterest {
                x: 0,
                y: 0,
                width: 512,
                height: 512,
            },
        })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ ROI configured\n");

    // Test 3: Set binning
    println!("Test 3: Set binning to 2x2");
    actor
        .ask(SetBinning {
            binning: BinningConfig { x_bin: 2, y_bin: 2 },
        })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Binning set\n");

    // Test 4: Set gain
    println!("Test 4: Set gain to 10");
    actor
        .ask(SetGain { gain: 10 })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Gain set\n");

    // Test 5: Set timing
    println!("Test 5: Set exposure to 50ms");
    actor
        .ask(SetTiming {
            timing: CameraTiming {
                exposure_us: 50_000,
                frame_period_ms: 55.0,
                trigger_mode: TriggerMode::Internal,
            },
        })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Timing configured\n");

    // Test 6: Snap frame
    println!("Test 6: Snap single frame");
    let frame = actor
        .ask(SnapFrame {
            timing: CameraTiming {
                exposure_us: 50_000,
                frame_period_ms: 55.0,
                trigger_mode: TriggerMode::Internal,
            },
        })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!(
        "  Frame: {}x{}, {} bytes, format: {:?}",
        frame.width,
        frame.height,
        frame.pixel_data.len(),
        frame.pixel_format
    );
    println!("  Frame number: {}\n", frame.frame_number);

    // Test 7: Start streaming
    println!("Test 7: Start streaming acquisition");
    let mut rx = actor
        .ask(StartStream {
            config: v4_daq::traits::camera_sensor::CameraStreamConfig {
                roi: RegionOfInterest {
                    x: 0,
                    y: 0,
                    width: 256,
                    height: 256,
                },
                binning: BinningConfig { x_bin: 4, y_bin: 4 },
                timing: CameraTiming {
                    exposure_us: 20_000,
                    frame_period_ms: 25.0,
                    trigger_mode: TriggerMode::Internal,
                },
                gain: 5,
            },
        })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Streaming started\n");

    // Test 8: Receive frames
    println!("Test 8: Receive 5 frames");
    for i in 0..5 {
        let frame = rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Frame channel closed"))?;
        println!(
            "  Frame {}: {}x{}, frame_number={}",
            i + 1,
            frame.width,
            frame.height,
            frame.frame_number
        );
    }
    println!();

    // Test 9: Stop streaming
    println!("Test 9: Stop streaming");
    actor
        .ask(StopStream)
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Streaming stopped\n");

    println!("=== Test Complete ===");

    actor.kill();
    actor.wait_for_shutdown().await;

    Ok(())
}
