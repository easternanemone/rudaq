//! CLI `snapshot` subcommand — capture a single frame from a running daemon.
//!
//! Connects to a daemon via gRPC, starts camera acquisition, captures one frame,
//! decompresses LZ4 if needed, and writes to disk as TIFF/PNG/raw.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use protocol::daq::{
    hardware_service_client::HardwareServiceClient, SetExposureRequest, StartStreamRequest,
    StopStreamRequest, StreamFramesRequest, StreamQuality,
};

/// Maximum gRPC message size (64 MB, matches server config).
const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// Output format for captured frames.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum SnapshotFormat {
    /// 16-bit TIFF (lossless, recommended for calibration)
    Tiff,
    /// 8-bit PNG (lossy for >8-bit cameras, good for quick preview)
    Png,
    /// Raw pixel bytes (no header, dimensions printed to stdout)
    Raw,
}

/// Capture a single frame from a camera device on a running daemon.
pub async fn handle_snapshot(
    device_id: String,
    output: PathBuf,
    exposure_ms: Option<f64>,
    format: SnapshotFormat,
    addr: String,
) -> Result<()> {
    println!("Connecting to daemon at {addr}...");

    let channel = tonic::transport::Channel::from_shared(addr.clone())?
        .connect()
        .await
        .with_context(|| format!("Failed to connect to daemon at {addr}"))?;

    let mut client =
        HardwareServiceClient::new(channel).max_decoding_message_size(MAX_MESSAGE_SIZE);

    // Optionally set exposure before capture via ExposureControl RPC
    if let Some(ms) = exposure_ms {
        println!("Setting exposure to {ms:.1} ms...");
        client
            .set_exposure(SetExposureRequest {
                device_id: device_id.clone(),
                exposure_ms: ms,
            })
            .await
            .context("Failed to set exposure time")?;
    }

    // Start camera acquisition
    println!("Starting acquisition on {device_id}...");
    client
        .start_stream(StartStreamRequest {
            device_id: device_id.clone(),
            frame_count: None,
        })
        .await
        .context("Failed to start camera stream")?;

    // Subscribe to frame stream (max_fps=0 = no limit, quality=Full)
    let mut stream = client
        .stream_frames(StreamFramesRequest {
            device_id: device_id.clone(),
            max_fps: 0,
            quality: StreamQuality::Full.into(),
        })
        .await
        .context("Failed to open frame stream")?
        .into_inner();

    // Capture first frame
    println!("Waiting for frame...");
    let mut frame = stream
        .message()
        .await
        .map_err(|e| anyhow::anyhow!("Frame stream error: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("Stream ended without delivering a frame"))?;

    // Stop camera (best-effort — don't fail if already stopped or owned by another client)
    let _ = client
        .stop_stream(StopStreamRequest {
            device_id: device_id.clone(),
        })
        .await;

    // Decompress LZ4 if needed
    if frame.compression != 0 {
        protocol::compression::decompress_frame(&mut frame)
            .map_err(|e| anyhow::anyhow!("Decompression failed: {e}"))?;
    }

    // Mono12 frames are unpacked to u16 server-side (bd-q2n6),
    // so treat 12-bit as 16-bit for file writing purposes.
    let effective_bit_depth = if frame.bit_depth == 12 {
        16
    } else {
        frame.bit_depth
    };

    write_frame_to_file(
        &frame.data,
        frame.width,
        frame.height,
        effective_bit_depth,
        &output,
        format,
    )?;

    let size_kb = std::fs::metadata(&output)
        .map(|m| m.len() / 1024)
        .unwrap_or(0);
    println!(
        "Captured {}x{} {}bit frame -> {} ({size_kb} KB)",
        frame.width,
        frame.height,
        frame.bit_depth,
        output.display()
    );

    Ok(())
}

/// Write raw pixel data to disk in the requested format.
fn write_frame_to_file(
    data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    output: &Path,
    format: SnapshotFormat,
) -> Result<()> {
    match format {
        SnapshotFormat::Tiff => {
            let file = File::create(output)
                .with_context(|| format!("Cannot create {}", output.display()))?;
            let writer = BufWriter::new(file);
            let encoder = image::codecs::tiff::TiffEncoder::new(writer);

            let color_type = if bit_depth >= 16 {
                image::ExtendedColorType::L16
            } else {
                image::ExtendedColorType::L8
            };

            encoder
                .encode(data, width, height, color_type)
                .context("TIFF encoding failed")?;
        }
        SnapshotFormat::Png => {
            if bit_depth >= 16 {
                // Scale 16-bit down to 8-bit for PNG
                let pixels_8bit: Vec<u8> = data
                    .chunks_exact(2)
                    .map(|c| (u16::from_le_bytes([c[0], c[1]]) >> 8) as u8)
                    .collect();
                let img = image::GrayImage::from_raw(width, height, pixels_8bit)
                    .ok_or_else(|| anyhow::anyhow!("Frame dimensions don't match data length"))?;
                img.save(output).context("PNG save failed")?;
            } else {
                let img = image::GrayImage::from_raw(width, height, data.to_vec())
                    .ok_or_else(|| anyhow::anyhow!("Frame dimensions don't match data length"))?;
                img.save(output).context("PNG save failed")?;
            }
        }
        SnapshotFormat::Raw => {
            std::fs::write(output, data)
                .with_context(|| format!("Failed to write raw data to {}", output.display()))?;
        }
    }
    Ok(())
}
