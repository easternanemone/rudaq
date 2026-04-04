#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::print_stdout)]

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use protocol::compression::decompress_frame_into;
use protocol::daq::hardware_service_client::HardwareServiceClient;
use protocol::daq::{
    FrameData, ListDevicesRequest, StartStreamRequest, StopStreamRequest, StreamFramesRequest,
    StreamQuality,
};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::{Instant, sleep, timeout};
use tokio_stream::StreamExt;
use tonic::transport::{Channel, Endpoint};

#[derive(Debug, Parser)]
#[command(
    name = "hardware-smoke",
    about = "Smoke-test a deployed rust-daq daemon against real hardware"
)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:50051")]
    addr: String,

    #[arg(long)]
    camera_id: String,

    #[arg(long = "expected-device")]
    expected_devices: Vec<String>,

    #[arg(long, default_value_t = 5)]
    frames: u32,

    #[arg(long, default_value_t = 0)]
    max_fps: u32,

    #[arg(long, default_value_t = 90)]
    ready_timeout_secs: u64,

    #[arg(long, default_value_t = 30)]
    frame_timeout_secs: u64,

    #[arg(long, default_value_t = 5)]
    connect_timeout_secs: u64,

    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct SmokeSummary {
    success: bool,
    addr: String,
    camera_id: String,
    expected_devices: Vec<String>,
    discovered_devices: Vec<String>,
    registration_failures: Vec<String>,
    frames_requested: u32,
    frames_received: u32,
    first_frame_number: Option<u64>,
    last_frame_number: Option<u64>,
    gap_events: u64,
    first_timestamp_ns: Option<u64>,
    last_timestamp_ns: Option<u64>,
    frame_shape: Option<FrameShape>,
    notes: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
struct FrameShape {
    width: u32,
    height: u32,
    bit_depth: u32,
    bytes_per_frame: usize,
}

#[derive(Debug, Default)]
struct FrameStats {
    frames_received: u32,
    first_frame_number: Option<u64>,
    last_frame_number: Option<u64>,
    gap_events: u64,
    first_timestamp_ns: Option<u64>,
    last_timestamp_ns: Option<u64>,
    frame_shape: Option<FrameShape>,
}

#[derive(Debug)]
struct InventorySnapshot {
    discovered_devices: Vec<String>,
    registration_failures: Vec<String>,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err:?}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = Args::parse();
    let mut summary = SmokeSummary {
        success: false,
        addr: args.addr.clone(),
        camera_id: args.camera_id.clone(),
        expected_devices: args.expected_devices.clone(),
        discovered_devices: Vec::new(),
        registration_failures: Vec::new(),
        frames_requested: args.frames,
        frames_received: 0,
        first_frame_number: None,
        last_frame_number: None,
        gap_events: 0,
        first_timestamp_ns: None,
        last_timestamp_ns: None,
        frame_shape: None,
        notes: Vec::new(),
        error: None,
    };

    let outcome = run_inner(&args, &mut summary).await;
    match &outcome {
        Ok(()) => summary.success = true,
        Err(err) => summary.error = Some(err.to_string()),
    }

    if let Some(output_path) = &args.output {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(&summary)?;
        fs::write(output_path, json)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
    }

    println!("{}", serde_json::to_string_pretty(&summary)?);
    outcome
}

async fn run_inner(args: &Args, summary: &mut SmokeSummary) -> Result<()> {
    let inventory = wait_for_inventory(args).await?;
    summary.discovered_devices = inventory.discovered_devices;
    summary.registration_failures = inventory.registration_failures;

    let channel = connect_channel(&args.addr, args.connect_timeout_secs).await?;
    let mut control = HardwareServiceClient::new(channel.clone());
    let mut streaming = HardwareServiceClient::new(channel);

    control
        .start_stream(StartStreamRequest {
            device_id: args.camera_id.clone(),
            frame_count: Some(args.frames),
        })
        .await
        .with_context(|| format!("failed to start stream for {}", args.camera_id))?;

    let collect_result = collect_frames(args, &mut streaming).await;

    match control
        .stop_stream(StopStreamRequest {
            device_id: args.camera_id.clone(),
        })
        .await
    {
        Ok(_) => summary
            .notes
            .push(format!("stop_stream acknowledged for {}", args.camera_id)),
        Err(err) => summary
            .notes
            .push(format!("stop_stream returned non-fatal error: {err}")),
    }

    let stats = collect_result?;
    summary.frames_received = stats.frames_received;
    summary.first_frame_number = stats.first_frame_number;
    summary.last_frame_number = stats.last_frame_number;
    summary.gap_events = stats.gap_events;
    summary.first_timestamp_ns = stats.first_timestamp_ns;
    summary.last_timestamp_ns = stats.last_timestamp_ns;
    summary.frame_shape = stats.frame_shape;

    Ok(())
}

async fn wait_for_inventory(args: &Args) -> Result<InventorySnapshot> {
    let deadline = Instant::now() + Duration::from_secs(args.ready_timeout_secs);
    let mut last_error: Option<anyhow::Error> = None;

    while Instant::now() < deadline {
        match fetch_inventory(args).await {
            Ok(snapshot) => {
                let missing = missing_devices(&snapshot.discovered_devices, &args.expected_devices);
                if missing.is_empty() {
                    return Ok(snapshot);
                }
                last_error = Some(anyhow!(
                    "expected device(s) not yet present: {}",
                    missing.join(", ")
                ));
            }
            Err(err) => {
                last_error = Some(err);
            }
        }

        sleep(Duration::from_secs(2)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow!("timed out waiting for daemon inventory")))
}

async fn fetch_inventory(args: &Args) -> Result<InventorySnapshot> {
    let channel = connect_channel(&args.addr, args.connect_timeout_secs).await?;
    let mut client = HardwareServiceClient::new(channel);
    let response = client
        .list_devices(ListDevicesRequest {
            capability_filter: None,
        })
        .await
        .context("list_devices failed")?
        .into_inner();

    Ok(InventorySnapshot {
        discovered_devices: response
            .devices
            .into_iter()
            .map(|device| device.id)
            .collect(),
        registration_failures: response
            .registration_failures
            .into_iter()
            .map(|failure| format!("{failure:?}"))
            .collect(),
    })
}

fn missing_devices(discovered: &[String], expected: &[String]) -> Vec<String> {
    expected
        .iter()
        .filter(|expected_device| !discovered.iter().any(|device| device == *expected_device))
        .cloned()
        .collect()
}

async fn connect_channel(addr: &str, connect_timeout_secs: u64) -> Result<Channel> {
    let endpoint = Endpoint::from_shared(addr.to_string())
        .with_context(|| format!("invalid daemon URL: {addr}"))?
        .connect_timeout(Duration::from_secs(connect_timeout_secs))
        .tcp_keepalive(Some(Duration::from_secs(30)));

    endpoint
        .connect()
        .await
        .with_context(|| format!("failed to connect to {addr}"))
}

async fn collect_frames(
    args: &Args,
    streaming: &mut HardwareServiceClient<Channel>,
) -> Result<FrameStats> {
    let response = streaming
        .stream_frames(StreamFramesRequest {
            device_id: args.camera_id.clone(),
            max_fps: args.max_fps,
            quality: StreamQuality::Full.into(),
        })
        .await
        .with_context(|| format!("failed to open frame stream for {}", args.camera_id))?;

    let mut stream = response.into_inner();
    let mut stats = FrameStats::default();
    let mut decompression_buffer = Vec::new();

    while stats.frames_received < args.frames {
        let maybe_frame = timeout(Duration::from_secs(args.frame_timeout_secs), stream.next())
            .await
            .with_context(|| {
                format!(
                    "timed out waiting for frame {} of {}",
                    stats.frames_received + 1,
                    args.frames
                )
            })?;

        let mut frame = match maybe_frame {
            Some(Ok(frame)) => frame,
            Some(Err(status)) => {
                return Err(anyhow!("stream returned gRPC error: {status}"));
            }
            None => {
                bail!(
                    "frame stream ended after {} frame(s), expected {}",
                    stats.frames_received,
                    args.frames
                );
            }
        };

        validate_frame(
            &mut frame,
            &mut decompression_buffer,
            &mut stats,
            &args.camera_id,
        )?;
    }

    Ok(stats)
}

fn validate_frame(
    frame: &mut FrameData,
    decompression_buffer: &mut Vec<u8>,
    stats: &mut FrameStats,
    expected_device_id: &str,
) -> Result<()> {
    decompress_frame_into(frame, decompression_buffer)
        .map_err(|err| anyhow!("frame decompression failed: {err}"))?;

    if frame.device_id != expected_device_id {
        bail!(
            "received frame for unexpected device {} (expected {})",
            frame.device_id,
            expected_device_id
        );
    }

    if frame.width == 0 || frame.height == 0 {
        bail!(
            "received empty frame dimensions {}x{}",
            frame.width,
            frame.height
        );
    }

    if frame.bit_depth == 0 {
        bail!("received frame with bit_depth=0");
    }

    let bytes_per_pixel = if frame.bit_depth <= 8 { 1usize } else { 2usize };
    let expected_len = frame.width as usize * frame.height as usize * bytes_per_pixel;
    if frame.data.len() != expected_len {
        bail!(
            "frame payload length mismatch: got {} bytes, expected {} for {}x{} @ {}-bit",
            frame.data.len(),
            expected_len,
            frame.width,
            frame.height,
            frame.bit_depth
        );
    }

    if frame.timestamp_ns == 0 {
        bail!("frame timestamp_ns was zero");
    }

    let current_shape = FrameShape {
        width: frame.width,
        height: frame.height,
        bit_depth: frame.bit_depth,
        bytes_per_frame: expected_len,
    };

    if let Some(previous_shape) = &stats.frame_shape {
        if previous_shape != &current_shape {
            bail!("frame shape changed from {previous_shape:?} to {current_shape:?}");
        }
    } else {
        stats.frame_shape = Some(current_shape);
    }

    if let Some(previous_number) = stats.last_frame_number {
        if frame.frame_number <= previous_number {
            bail!(
                "frame number did not advance monotonically: prev={}, current={}",
                previous_number,
                frame.frame_number
            );
        }
        if frame.frame_number > previous_number + 1 {
            stats.gap_events += 1;
            bail!(
                "frame gap detected: prev={}, current={}",
                previous_number,
                frame.frame_number
            );
        }
    } else {
        stats.first_frame_number = Some(frame.frame_number);
        stats.first_timestamp_ns = Some(frame.timestamp_ns);
    }

    stats.frames_received += 1;
    stats.last_frame_number = Some(frame.frame_number);
    stats.last_timestamp_ns = Some(frame.timestamp_ns);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::missing_devices;

    #[test]
    fn missing_devices_reports_only_absent_entries() {
        let discovered = vec!["prime_bsi".to_string(), "power_meter".to_string()];
        let expected = vec![
            "prime_bsi".to_string(),
            "maitai".to_string(),
            "power_meter".to_string(),
        ];

        assert_eq!(missing_devices(&discovered, &expected), vec!["maitai"]);
    }
}
