#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::print_stdout)]

#[cfg(not(feature = "server"))]
compile_error!("pvcam_grpc_harness requires the 'server' feature");

use anyhow::{Context, Result};
use daq_core::health::SystemHealthMonitor;
use daq_proto::daq::hardware_service_client::HardwareServiceClient;
use daq_proto::daq::{
    SetParameterRequest, StartStreamRequest, StopStreamRequest, StreamFramesRequest, StreamQuality,
};
use rust_daq::hardware::registry::create_lab_registry;
use serde::Serialize;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;
use tokio_stream::StreamExt;

#[derive(Clone, Debug)]
struct RoiConfig {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
struct HarnessConfig {
    scenario: String,
    duration: Duration,
    exposure_ms: f64,
    max_fps: u32,
    camera_id: String,
    addr: String,
    start_server: bool,
    multi_client: bool,
    param_churn: bool,
    roi: Option<RoiConfig>,
    binning: Option<(u16, u16)>,
    output_path: Option<String>,
}

#[derive(Debug, Default)]
struct StreamStats {
    frames_received: u64,
    frames_sent: u64,
    frames_dropped: u64,
    bytes_uncompressed: u64,
    bytes_compressed: u64,
    avg_latency_ms: f64,
    latency_samples: u64,
    max_latency_ms: f64,
    min_latency_ms: f64,
    last_frame_number: Option<u64>,
    gap_frames: u64,
    gap_events: u64,
    stall_events: u64,
}

#[derive(Debug, Serialize)]
struct SummaryMetrics {
    frames_received: u64,
    frames_sent: u64,
    frames_dropped: u64,
    drop_rate: f64,
    avg_latency_ms: f64,
    max_latency_ms: f64,
    min_latency_ms: f64,
    fps: f64,
    bytes_uncompressed: u64,
    bytes_compressed: u64,
    compression_ratio: f64,
    gap_events: u64,
    gap_frames: u64,
    stall_events: u64,
}

#[derive(Debug, Serialize)]
struct SystemMetrics {
    load_avg_1: Option<f64>,
    load_avg_5: Option<f64>,
    load_avg_15: Option<f64>,
    mem_total_kb: Option<u64>,
    mem_available_kb: Option<u64>,
    process_rss_kb: Option<u64>,
}

#[derive(Debug, Serialize)]
struct HarnessSummary {
    scenario: String,
    camera_id: String,
    addr: String,
    duration_secs: u64,
    exposure_ms: f64,
    max_fps: u32,
    start_time_unix_ns: u64,
    end_time_unix_ns: u64,
    metrics: SummaryMetrics,
    system_start: SystemMetrics,
    system_end: SystemMetrics,
    param_updates: Vec<ParamUpdateResult>,
    notes: Vec<String>,
    success: bool,
}

#[derive(Debug, Serialize)]
struct ParamUpdateResult {
    name: String,
    value: String,
    success: bool,
    error: Option<String>,
}

fn usage() {
    println!(
        "PVCAM gRPC harness\n\
Usage:\n\
  pvcam_grpc_harness [options]\n\
\n\
Options:\n\
  --scenario <baseline|stress|multiclient|param-churn>\n\
  --duration-secs <seconds>\n\
  --exposure-ms <ms>\n\
  --max-fps <fps>\n\
  --camera-id <device_id>     (default: prime_bsi)\n\
  --addr <host:port>          (default: 127.0.0.1:50051)\n\
  --no-server                 (do not start server; connect to existing daemon)\n\
  --multi-client              (enable second StreamFrames client)\n\
  --param-churn               (apply parameter updates during streaming)\n\
  --roi <x,y,w,h>\n\
  --binning <x,y>\n\
  --output <path>             (write JSON summary)\n\
  --help\n\
"
    );
}

fn parse_args() -> Result<HarnessConfig> {
    let mut scenario = "baseline".to_string();
    let mut duration = Duration::from_secs(1800);
    let mut exposure_ms = 100.0;
    let mut max_fps = 0;
    let mut camera_id = "prime_bsi".to_string();
    let mut addr = "127.0.0.1:50051".to_string();
    let mut start_server = true;
    let mut multi_client = false;
    let mut param_churn = false;
    let mut roi: Option<RoiConfig> = None;
    let mut binning: Option<(u16, u16)> = None;
    let mut output_path: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => scenario = args.next().context("Missing --scenario value")?,
            "--duration-secs" => {
                let value = args.next().context("Missing --duration-secs value")?;
                duration = Duration::from_secs(value.parse()?);
            }
            "--exposure-ms" => {
                let value = args.next().context("Missing --exposure-ms value")?;
                exposure_ms = value.parse()?;
            }
            "--max-fps" => {
                let value = args.next().context("Missing --max-fps value")?;
                max_fps = value.parse()?;
            }
            "--camera-id" => camera_id = args.next().context("Missing --camera-id value")?,
            "--addr" => addr = args.next().context("Missing --addr value")?,
            "--no-server" => start_server = false,
            "--multi-client" => multi_client = true,
            "--param-churn" => param_churn = true,
            "--roi" => {
                let value = args.next().context("Missing --roi value")?;
                roi = Some(parse_roi(&value)?);
            }
            "--binning" => {
                let value = args.next().context("Missing --binning value")?;
                binning = Some(parse_binning(&value)?);
            }
            "--output" => output_path = Some(args.next().context("Missing --output value")?),
            "--help" => {
                usage();
                std::process::exit(0);
            }
            _ => return Err(anyhow::anyhow!("Unknown argument: {}", arg)),
        }
    }

    apply_scenario_defaults(
        &scenario,
        &mut duration,
        &mut exposure_ms,
        &mut max_fps,
        &mut multi_client,
        &mut param_churn,
        &mut roi,
        &mut binning,
    )?;

    Ok(HarnessConfig {
        scenario,
        duration,
        exposure_ms,
        max_fps,
        camera_id,
        addr,
        start_server,
        multi_client,
        param_churn,
        roi,
        binning,
        output_path,
    })
}

fn apply_scenario_defaults(
    scenario: &str,
    duration: &mut Duration,
    exposure_ms: &mut f64,
    max_fps: &mut u32,
    multi_client: &mut bool,
    param_churn: &mut bool,
    roi: &mut Option<RoiConfig>,
    binning: &mut Option<(u16, u16)>,
) -> Result<()> {
    match scenario {
        "baseline" => {}
        "stress" => {
            *duration = Duration::from_secs(300);
            *exposure_ms = 10.0;
            *max_fps = 0;
            if roi.is_none() {
                *roi = Some(RoiConfig {
                    x: 0,
                    y: 0,
                    width: 512,
                    height: 512,
                });
            }
        }
        "multiclient" => {
            *duration = Duration::from_secs(600);
            *exposure_ms = 100.0;
            *multi_client = true;
        }
        "param-churn" => {
            *duration = Duration::from_secs(900);
            *exposure_ms = 100.0;
            *param_churn = true;
        }
        _ => return Err(anyhow::anyhow!("Unknown scenario '{}'", scenario)),
    }

    if binning.is_none() {
        *binning = Some((1, 1));
    }

    Ok(())
}

fn parse_roi(value: &str) -> Result<RoiConfig> {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 4 {
        return Err(anyhow::anyhow!("ROI must be x,y,width,height"));
    }
    Ok(RoiConfig {
        x: parts[0].parse()?,
        y: parts[1].parse()?,
        width: parts[2].parse()?,
        height: parts[3].parse()?,
    })
}

fn parse_binning(value: &str) -> Result<(u16, u16)> {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!("Binning must be x,y"));
    }
    Ok((parts[0].parse()?, parts[1].parse()?))
}

fn now_unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn read_system_metrics() -> SystemMetrics {
    let mut metrics = SystemMetrics {
        load_avg_1: None,
        load_avg_5: None,
        load_avg_15: None,
        mem_total_kb: None,
        mem_available_kb: None,
        process_rss_kb: None,
    };

    if let Ok(loadavg) = fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = loadavg.split_whitespace().collect();
        if parts.len() >= 3 {
            metrics.load_avg_1 = parts[0].parse().ok();
            metrics.load_avg_5 = parts[1].parse().ok();
            metrics.load_avg_15 = parts[2].parse().ok();
        }
    }

    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                metrics.mem_total_kb = line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
            }
            if line.starts_with("MemAvailable:") {
                metrics.mem_available_kb =
                    line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
            }
        }
    }

    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                metrics.process_rss_kb =
                    line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
                break;
            }
        }
    }

    metrics
}

async fn set_parameter(
    client: &mut HardwareServiceClient<tonic::transport::Channel>,
    device_id: &str,
    name: &str,
    value: &str,
) -> ParamUpdateResult {
    let request = SetParameterRequest {
        device_id: device_id.to_string(),
        parameter_name: name.to_string(),
        value: value.to_string(),
    };

    match client.set_parameter(request).await {
        Ok(resp) => ParamUpdateResult {
            name: name.to_string(),
            value: resp.into_inner().actual_value,
            success: true,
            error: None,
        },
        Err(err) => ParamUpdateResult {
            name: name.to_string(),
            value: value.to_string(),
            success: false,
            error: Some(err.to_string()),
        },
    }
}

fn update_latency(stats: &mut StreamStats, latency_ms: f64) {
    stats.latency_samples += 1;
    stats.avg_latency_ms += (latency_ms - stats.avg_latency_ms) / stats.latency_samples as f64;
    if latency_ms > stats.max_latency_ms {
        stats.max_latency_ms = latency_ms;
    }
    if stats.min_latency_ms == 0.0 || latency_ms < stats.min_latency_ms {
        stats.min_latency_ms = latency_ms;
    }
}

async fn run_stream(
    mut client: HardwareServiceClient<tonic::transport::Channel>,
    config: &HarnessConfig,
    stats: &mut StreamStats,
    notes: &mut Vec<String>,
    param_updates: &mut Vec<ParamUpdateResult>,
) -> Result<()> {
    let start_request = StartStreamRequest {
        device_id: config.camera_id.clone(),
        frame_count: None,
    };
    client
        .start_stream(start_request)
        .await
        .context("StartStream failed")?;

    let stream_request = StreamFramesRequest {
        device_id: config.camera_id.clone(),
        max_fps: config.max_fps,
        quality: StreamQuality::Full.into(), // Full resolution for harness testing
    };
    let mut stream = client
        .stream_frames(stream_request)
        .await
        .context("StreamFrames failed")?
        .into_inner();

    let mut last_frame_time = Instant::now();
    let start_time = Instant::now();
    let mut metrics_interval = tokio::time::interval(Duration::from_secs(60));
    let mut ended_due_to_error = false;

    let mut param_handle = None;
    if config.param_churn {
        let config_clone = config.clone();
        param_handle = Some(tokio::spawn(
            async move { run_param_churn(&config_clone).await },
        ));
        notes.push("Parameter churn task started".to_string());
    }

    let mut secondary_handle = None;
    if config.multi_client {
        let config_clone = config.clone();
        let duration_secs = config.duration.as_secs();
        let secondary_secs = std::cmp::min(60, std::cmp::max(20, duration_secs / 10));
        secondary_handle = Some(tokio::spawn(async move {
            run_secondary_client(&config_clone, Duration::from_secs(secondary_secs)).await
        }));
        notes.push("Secondary client task started".to_string());
    }

    while start_time.elapsed() < config.duration {
        tokio::select! {
            _ = metrics_interval.tick() => {
                notes.push(format!(
                    "Progress: {} frames, {} dropped, {:.1} avg latency ms",
                    stats.frames_received,
                    stats.frames_dropped,
                    stats.avg_latency_ms
                ));
            }
            result = stream.next() => {
                match result {
                    Some(Ok(frame)) => {
                        last_frame_time = Instant::now();
                        stats.frames_received += 1;

                        if let Some(metrics) = frame.metrics {
                            stats.frames_sent = metrics.frames_sent;
                            stats.frames_dropped = metrics.frames_dropped;
                            if metrics.avg_latency_ms > 0.0 {
                                update_latency(stats, metrics.avg_latency_ms);
                            }
                        }

                        let uncompressed_size = if frame.uncompressed_size > 0 {
                            frame.uncompressed_size as u64
                        } else {
                            frame.data.len() as u64
                        };
                        stats.bytes_uncompressed += uncompressed_size;
                        stats.bytes_compressed += frame.data.len() as u64;

                        if frame.timestamp_ns > 0 {
                            let now_ns = now_unix_ns();
                            let latency_ms = (now_ns.saturating_sub(frame.timestamp_ns) as f64) / 1_000_000.0;
                            update_latency(stats, latency_ms);
                        }

                        if let Some(prev) = stats.last_frame_number {
                            if frame.frame_number > prev + 1 {
                                stats.gap_events += 1;
                                stats.gap_frames += frame.frame_number - prev - 1;
                            }
                        }
                        stats.last_frame_number = Some(frame.frame_number);
                    }
                    Some(Err(err)) => {
                        notes.push(format!("Stream error: {}", err));
                        ended_due_to_error = true;
                        break;
                    }
                    None => {
                        notes.push("Stream ended unexpectedly".to_string());
                        ended_due_to_error = true;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                if last_frame_time.elapsed() > Duration::from_secs(2) {
                    stats.stall_events += 1;
                    notes.push("Stall detected: no frames for >2s".to_string());
                }
            }
        }
    }

    if ended_due_to_error {
        if let Some(handle) = param_handle {
            handle.abort();
            notes.push("Parameter churn task aborted due to stream error".to_string());
        }
        if let Some(handle) = secondary_handle {
            handle.abort();
            notes.push("Secondary client task aborted due to stream error".to_string());
        }
    } else {
        if let Some(handle) = param_handle {
            if let Ok(update_results) = handle.await {
                param_updates.extend(update_results);
            } else {
                notes.push("Parameter churn task failed to join".to_string());
            }
        }

        if let Some(handle) = secondary_handle {
            match handle.await {
                Ok(outcome) => {
                    notes.push(format!(
                        "Secondary client frames: {} (status: {})",
                        outcome.frames_received, outcome.status
                    ));
                }
                Err(_) => notes.push("Secondary client task failed to join".to_string()),
            }
        }
    }

    let stop_request = StopStreamRequest {
        device_id: config.camera_id.clone(),
    };
    client
        .stop_stream(stop_request)
        .await
        .context("StopStream failed")?;

    Ok(())
}

async fn run_param_churn(config: &HarnessConfig) -> Vec<ParamUpdateResult> {
    let mut results = Vec::new();
    let addr = format!("http://{}", config.addr);
    let mut client = match HardwareServiceClient::connect(addr).await {
        Ok(client) => client,
        Err(err) => {
            results.push(ParamUpdateResult {
                name: "param_churn".to_string(),
                value: "connect".to_string(),
                success: false,
                error: Some(err.to_string()),
            });
            return results;
        }
    };

    let deadline = Instant::now() + config.duration;
    let continue_sleep = |duration: Duration| async move {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        if now + duration >= deadline {
            tokio::time::sleep_until(deadline).await;
            return false;
        }
        tokio::time::sleep(duration).await;
        true
    };

    if !continue_sleep(Duration::from_secs(60)).await {
        return results;
    }

    let exposure_values = [
        config.exposure_ms,
        (config.exposure_ms / 2.0).max(1.0),
        config.exposure_ms,
    ];
    for exposure in exposure_values {
        results.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.exposure_ms",
                &format!("{}", exposure),
            )
            .await,
        );
        if !continue_sleep(Duration::from_secs(120)).await {
            return results;
        }
    }

    if let Some(roi) = &config.roi {
        let quarter_roi = RoiConfig {
            x: roi.x,
            y: roi.y,
            width: roi.width / 2,
            height: roi.height / 2,
        };
        results.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.roi",
                &format!(
                    "{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                    quarter_roi.x, quarter_roi.y, quarter_roi.width, quarter_roi.height
                ),
            )
            .await,
        );
        if !continue_sleep(Duration::from_secs(120)).await {
            return results;
        }
        results.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.roi",
                &format!(
                    "{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                    roi.x, roi.y, roi.width, roi.height
                ),
            )
            .await,
        );
    }

    if let Some((bx, by)) = config.binning {
        let churn_bx = std::cmp::max(2u16, bx);
        let churn_by = std::cmp::max(2u16, by);
        results.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.binning",
                &format!("[{},{}]", churn_bx, churn_by),
            )
            .await,
        );
        if !continue_sleep(Duration::from_secs(120)).await {
            return results;
        }
        results.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.binning",
                &format!("[{},{}]", bx, by),
            )
            .await,
        );
    }

    results
}

struct SecondaryOutcome {
    frames_received: u64,
    status: String,
}

async fn run_secondary_client(config: &HarnessConfig, duration: Duration) -> SecondaryOutcome {
    let mut frames = 0u64;
    let mut status = Vec::new();
    let phases = [
        ("initial", duration),
        ("reconnect", Duration::from_secs(10)),
    ];

    for (label, phase_duration) in phases {
        let addr = format!("http://{}", config.addr);
        let mut client = match HardwareServiceClient::connect(addr).await {
            Ok(client) => client,
            Err(err) => {
                status.push(format!("{} connect failed: {}", label, err));
                break;
            }
        };

        let request = StreamFramesRequest {
            device_id: config.camera_id.clone(),
            max_fps: config.max_fps,
            quality: StreamQuality::Full.into(),
        };

        let mut stream = match client.stream_frames(request).await {
            Ok(resp) => resp.into_inner(),
            Err(err) => {
                status.push(format!("{} stream failed: {}", label, err));
                break;
            }
        };

        let start = Instant::now();
        while start.elapsed() < phase_duration {
            if stream.next().await.is_some() {
                frames += 1;
            }
        }

        status.push(format!("{} phase complete", label));
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let status = if status.is_empty() {
        "no activity".to_string()
    } else {
        status.join("; ")
    };

    SecondaryOutcome {
        frames_received: frames,
        status,
    }
}

fn compute_summary(
    config: &HarnessConfig,
    stats: &StreamStats,
    start_time_ns: u64,
    end_time_ns: u64,
    system_start: SystemMetrics,
    system_end: SystemMetrics,
    param_updates: Vec<ParamUpdateResult>,
    notes: Vec<String>,
) -> HarnessSummary {
    let duration_secs = config.duration.as_secs_f64().max(1.0);
    let fps = stats.frames_received as f64 / duration_secs;
    let drop_rate = if stats.frames_sent > 0 {
        stats.frames_dropped as f64 / stats.frames_sent as f64
    } else {
        0.0
    };
    let compression_ratio = if stats.bytes_compressed > 0 {
        stats.bytes_uncompressed as f64 / stats.bytes_compressed as f64
    } else {
        1.0
    };

    let metrics = SummaryMetrics {
        frames_received: stats.frames_received,
        frames_sent: stats.frames_sent,
        frames_dropped: stats.frames_dropped,
        drop_rate,
        avg_latency_ms: stats.avg_latency_ms,
        max_latency_ms: stats.max_latency_ms,
        min_latency_ms: stats.min_latency_ms,
        fps,
        bytes_uncompressed: stats.bytes_uncompressed,
        bytes_compressed: stats.bytes_compressed,
        compression_ratio,
        gap_events: stats.gap_events,
        gap_frames: stats.gap_frames,
        stall_events: stats.stall_events,
    };

    let success = evaluate_success(config, &metrics, &notes);

    HarnessSummary {
        scenario: config.scenario.clone(),
        camera_id: config.camera_id.clone(),
        addr: config.addr.clone(),
        duration_secs: config.duration.as_secs(),
        exposure_ms: config.exposure_ms,
        max_fps: config.max_fps,
        start_time_unix_ns: start_time_ns,
        end_time_unix_ns: end_time_ns,
        metrics,
        system_start,
        system_end,
        param_updates,
        notes,
        success,
    }
}

fn evaluate_success(config: &HarnessConfig, metrics: &SummaryMetrics, notes: &[String]) -> bool {
    let expected_fps = if config.exposure_ms > 0.0 {
        1000.0 / config.exposure_ms
    } else {
        0.0
    };
    let expected_fps = if config.max_fps > 0 {
        expected_fps.min(config.max_fps as f64)
    } else {
        expected_fps
    };

    let min_fps_factor = match config.scenario.as_str() {
        "stress" => 0.4,
        _ => 0.8,
    };
    let max_drop_rate = match config.scenario.as_str() {
        "stress" => 0.5,
        _ => 0.05,
    };

    let fps_ok = metrics.fps >= expected_fps * min_fps_factor;
    let drop_ok = metrics.drop_rate <= max_drop_rate;
    let stall_ok = metrics.stall_events == 0;
    let error_notes = notes.iter().any(|note| note.contains("Stream error"));

    fps_ok && drop_ok && stall_ok && !error_notes
}

async fn spawn_server(addr: &str) -> Result<()> {
    let registry = create_lab_registry().await?;
    let health_monitor = Arc::new(SystemHealthMonitor::new(Default::default()));
    let addr = addr.parse().context("Invalid server address")?;

    tokio::spawn(async move {
        if let Err(err) = daq_server::grpc::server::start_server_with_hardware(
            addr,
            Arc::new(registry),
            health_monitor,
        )
        .await
        {
            eprintln!("Server error: {}", err);
        }
    });

    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let config = parse_args()?;

    println!("PVCAM gRPC harness starting: {}", config.scenario);
    println!("  camera: {}", config.camera_id);
    println!("  addr: {}", config.addr);
    println!("  duration: {}s", config.duration.as_secs());
    println!("  exposure_ms: {}", config.exposure_ms);
    println!("  max_fps: {}", config.max_fps);
    println!("  start_server: {}", config.start_server);

    if config.start_server {
        spawn_server(&config.addr).await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    let system_start = read_system_metrics();
    let start_time_ns = now_unix_ns();

    let addr = format!("http://{}", config.addr);
    let mut client = HardwareServiceClient::connect(addr.clone())
        .await
        .context("Failed to connect to gRPC server")?;

    let mut param_updates = Vec::new();
    param_updates.push(
        set_parameter(
            &mut client,
            &config.camera_id,
            "acquisition.exposure_ms",
            &format!("{}", config.exposure_ms),
        )
        .await,
    );

    if let Some(roi) = &config.roi {
        param_updates.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.roi",
                &format!(
                    "{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                    roi.x, roi.y, roi.width, roi.height
                ),
            )
            .await,
        );
    }

    if let Some((bx, by)) = config.binning {
        param_updates.push(
            set_parameter(
                &mut client,
                &config.camera_id,
                "acquisition.binning",
                &format!("[{},{}]", bx, by),
            )
            .await,
        );
    }

    let mut notes = Vec::new();
    let mut stats = StreamStats::default();
    run_stream(client, &config, &mut stats, &mut notes, &mut param_updates).await?;

    let end_time_ns = now_unix_ns();
    let system_end = read_system_metrics();

    let summary = compute_summary(
        &config,
        &stats,
        start_time_ns,
        end_time_ns,
        system_start,
        system_end,
        param_updates,
        notes,
    );

    println!("Harness complete. Success: {}", summary.success);
    println!(
        "Frames: {} received, {} dropped (rate {:.2}%)",
        summary.metrics.frames_received,
        summary.metrics.frames_dropped,
        summary.metrics.drop_rate * 100.0
    );
    println!("Avg latency: {:.2} ms", summary.metrics.avg_latency_ms);
    println!("FPS: {:.2}", summary.metrics.fps);

    if let Some(path) = &config.output_path {
        let json = serde_json::to_string_pretty(&summary)?;
        fs::write(path, json)?;
        println!("Summary written to {}", path);
    }

    if summary.success {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Harness failed pass criteria"))
    }
}
