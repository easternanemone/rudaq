//! Remote gRPC streaming test
//!
//! Tests frame streaming from a remote daemon to verify:
//! - Tailscale direct connectivity (no SSH tunnel needed)
//! - LZ4 compression working correctly
//! - Compression ratio metrics
//! - Real vs mock data detection
//!
//! Usage:
//! ```bash
//! # Connect to remote daemon (replace IP with your Tailscale address)
//! DAQ_DAEMON_URL=http://100.117.5.12:50051 cargo run --example remote_stream_test
//! ```

use protocol::daq::{
    hardware_service_client::HardwareServiceClient, ListDevicesRequest, ListParametersRequest,
    StartStreamRequest, StopStreamRequest, StreamFramesRequest, StreamQuality,
};
use futures::StreamExt;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("DAQ_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:50051".into());
    println!("Connecting to daemon at {}", endpoint);

    let channel = tonic::transport::Channel::from_shared(endpoint.clone())?
        .connect_timeout(std::time::Duration::from_secs(10))
        .connect()
        .await?;

    // Increase max message size for camera frames (100MB limit matches server)
    let mut client = HardwareServiceClient::new(channel)
        .max_decoding_message_size(100 * 1024 * 1024)
        .max_encoding_message_size(100 * 1024 * 1024);

    // List devices first
    let devices = client
        .list_devices(ListDevicesRequest {
            capability_filter: None,
        })
        .await?;
    println!("\nAvailable devices:");
    for d in &devices.get_ref().devices {
        println!("  - {} ({}) [frame_producer={}]", d.id, d.driver_type, d.is_frame_producer);
    }

    // Find the camera (is_frame_producer = true)
    let camera_id = devices
        .get_ref()
        .devices
        .iter()
        .find(|d| d.is_frame_producer || d.id == "camera")
        .map(|d| d.id.clone());

    let Some(cam) = camera_id else {
        println!("\nNo camera found. Available devices listed above.");
        return Ok(());
    };

    // Query camera parameters to check if real hardware
    println!("\nQuerying camera parameters...");
    let params_resp = client
        .list_parameters(ListParametersRequest {
            device_id: cam.clone(),
        })
        .await?;

    let params = &params_resp.get_ref().parameters;
    println!("Camera has {} parameters:", params.len());
    for p in params.iter().take(10) {
        println!("  {} ({}) - {}", p.name, p.dtype, p.description);
    }
    if params.len() > 10 {
        println!("  ... and {} more", params.len() - 10);
    }

    println!("\nStarting stream from {}...", cam);

    // Start stream (None for frame_count = continuous)
    client
        .start_stream(StartStreamRequest {
            device_id: cam.clone(),
            frame_count: None,
        })
        .await?;

    // Stream a few frames
    let request = StreamFramesRequest {
        device_id: cam.clone(),
        max_fps: 30, // Rate limit for GUI rendering
        quality: StreamQuality::Full.into(), // Full resolution for test
    };
    let mut stream = client.stream_frames(request).await?.into_inner();

    let mut count = 0;
    let mut total_uncompressed = 0usize;
    let mut total_compressed = 0usize;
    let mut prev_frame_number = 0u64;

    println!("\nReceiving frames (press Ctrl+C to stop):");
    println!("{}", "─".repeat(90));

    while let Some(frame_result) = stream.next().await {
        if count >= 10 {
            break;
        }
        match frame_result {
            Ok(frame) => {
                let compression_type = frame.compression;
                let compressed_size = frame.data.len();
                let uncompressed_size = frame.uncompressed_size as usize;
                let ratio = if compressed_size > 0 {
                    uncompressed_size as f64 / compressed_size as f64
                } else {
                    1.0
                };

                total_uncompressed += uncompressed_size;
                total_compressed += compressed_size;

                // Decompress to analyze pixel data
                let pixel_data = if compression_type == 1 {
                    // LZ4 compressed - decompress
                    lz4_flex::decompress_size_prepended(&frame.data).ok()
                } else {
                    Some(frame.data.clone())
                };

                // Analyze pixel statistics (16-bit data)
                let (min_val, max_val, avg_val, unique_count) = if let Some(ref data) = pixel_data {
                    if frame.bit_depth == 16 && data.len() >= 2 {
                        let pixels: Vec<u16> = data.chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        let min = *pixels.iter().min().unwrap_or(&0);
                        let max = *pixels.iter().max().unwrap_or(&0);
                        let sum: u64 = pixels.iter().map(|&x| x as u64).sum();
                        let avg = sum as f64 / pixels.len() as f64;
                        // Count unique values (sample first 10000 pixels)
                        let mut unique: std::collections::HashSet<u16> = std::collections::HashSet::new();
                        for &p in pixels.iter().take(10000) {
                            unique.insert(p);
                        }
                        (min, max, avg, unique.len())
                    } else {
                        (0, 0, 0.0, 0)
                    }
                } else {
                    (0, 0, 0.0, 0)
                };

                // Check for frame number gaps
                let gap = if prev_frame_number > 0 && frame.frame_number > prev_frame_number {
                    frame.frame_number - prev_frame_number - 1
                } else {
                    0
                };
                prev_frame_number = frame.frame_number;

                // compression=1 means LZ4, compression=0 means none
                let comp_str = if compression_type == 1 { "LZ4" } else { "none" };

                println!(
                    "Frame {:4}: {:4}x{:4}  {} {:6} KB → {:5} KB ({:5.1}x) | px: min={:5} max={:5} avg={:8.1} uniq={:5}{}",
                    frame.frame_number,
                    frame.width,
                    frame.height,
                    comp_str,
                    uncompressed_size / 1024,
                    compressed_size / 1024,
                    ratio,
                    min_val,
                    max_val,
                    avg_val,
                    unique_count,
                    if gap > 0 { format!(" [GAP:{}]", gap) } else { String::new() }
                );
                count += 1;
            }
            Err(e) => {
                println!("Stream error: {}", e);
                break;
            }
        }
    }

    println!("{}", "─".repeat(90));

    // Print summary
    if total_compressed > 0 {
        let overall_ratio = total_uncompressed as f64 / total_compressed as f64;
        println!(
            "\nSummary: {} frames, {} MB → {} MB ({:.1}x compression)",
            count,
            total_uncompressed / 1024 / 1024,
            total_compressed / 1024 / 1024,
            overall_ratio
        );
    }

    // Stop stream
    client
        .stop_stream(StopStreamRequest { device_id: cam })
        .await?;
    println!("Stream stopped");

    Ok(())
}
