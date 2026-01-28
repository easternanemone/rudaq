//! Rerun Visualization Sink
//!
//! Pushes measurements to Rerun.io for visualization.
//!
//! ## Operating Modes
//!
//! - **`new()`** / **`with_app_id()`**: Spawns a local viewer (for development)
//! - **`new_server()`**: Starts a gRPC server that remote viewers can connect to (for headless daemons)
//! - **`new_server_with_recording()`**: gRPC server + simultaneous .rrd file recording
//!
//! ## Remote Viewer Connection
//!
//! When using `new_server()`, remote viewers connect via:
//! ```text
//! rerun+http://{bind_ip}:{port}/proxy
//! ```
//!
//! ## Simultaneous Recording
//!
//! For headless daemons that need both live visualization AND persistent recording:
//! ```rust,ignore
//! let sink = RerunSink::new_server_with_recording(
//!     "0.0.0.0",
//!     9876,
//!     false,
//!     Some("experiment_001.rrd"),
//! )?;
//! ```
//!
//! This creates two parallel streams:
//! - gRPC server for live viewers (via `serve_grpc_opts()`)
//! - File sink for .rrd recording (via `FileSink`)
//!
//! ## Health Monitoring
//!
//! Use `start_heartbeat_task()` to enable periodic health logging:
//! ```rust,ignore
//! let sink = RerunSink::new_server("0.0.0.0", 9876, false)?;
//! let heartbeat_handle = sink.start_heartbeat_task(Duration::from_secs(5));
//! // Later: heartbeat_handle.abort() to stop
//! ```
//!
//! This logs to `/system/heartbeat` at the specified interval, allowing viewers
//! to detect if the server is alive and data is flowing.
//!
//! ## Flush Configuration
//!
//! Rerun batches data before sending. Configure via environment variables:
//! - `RERUN_FLUSH_TICK_SECS`: Flush interval (default: 0.008 = 8ms for real-time video)
//! - `RERUN_FLUSH_NUM_BYTES`: Flush when buffer exceeds N bytes (default: 1MB)
//! - `RERUN_FLUSH_NUM_ROWS`: Flush when buffer exceeds N rows
//!
//! For lower latency (e.g., 4ms): `RERUN_FLUSH_TICK_SECS=0.004`
//!
//! ## Blueprint Support
//!
//! Since the Rust Blueprint API is not yet available (see rerun-io/rerun#5521),
//! blueprints must be created using Python and loaded via `load_blueprint()`.
//!
//! Generate blueprints with:
//! ```bash
//! cd crates/daq-server/blueprints
//! pip install rerun-sdk
//! python generate_blueprints.py
//! ```
//!
//! Then load in Rust:
//! ```rust,ignore
//! let sink = RerunSink::new_server("0.0.0.0", 9876, false)?;
//! sink.load_blueprint("crates/daq-server/blueprints/daq_default.rbl")?;
//! ```

use anyhow::Result;
use async_trait::async_trait;
use common::core::{ImageMetadata, Measurement, PixelBuffer};
use common::pipeline::MeasurementSink;
use rerun::archetypes::{Scalars, Tensor};
use rerun::sink::FileSink;
use rerun::{MemoryLimit, RecordingStream, RecordingStreamBuilder, ServerOptions};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Default application ID - must match the Python blueprint generator
pub const APP_ID: &str = "rust-daq";

/// Default gRPC server port for Rerun
pub const DEFAULT_RERUN_PORT: u16 = 9876;

pub struct RerunSink {
    /// Primary stream for live visualization (gRPC server or spawned viewer)
    rec: RecordingStream,
    /// Optional secondary stream for simultaneous .rrd file recording
    recording_stream: Option<Arc<RecordingStream>>,
}

impl RerunSink {
    /// Create a new Rerun sink that spawns a viewer or connects to a remote one.
    ///
    /// This is useful for local development. For headless daemon mode, use `new_server()`.
    pub fn new() -> Result<Self> {
        Self::with_app_id(APP_ID)
    }

    /// Create a new Rerun sink with a custom application ID.
    ///
    /// Note: If using pre-generated blueprints, the app ID must match.
    /// This spawns a local viewer - for headless mode use `new_server()`.
    pub fn with_app_id(application_id: &str) -> Result<Self> {
        let rec = RecordingStreamBuilder::new(application_id)
            .spawn() // Spawns a viewer process or connects to one
            ?;
        Ok(Self {
            rec,
            recording_stream: None,
        })
    }

    /// Create a new Rerun sink that starts a gRPC server for remote viewers.
    ///
    /// Remote viewers (including embedded GUI viewers) connect via:
    /// `rerun+http://{bind_ip}:{port}/proxy`
    ///
    /// # Arguments
    /// * `bind_ip` - IP address to bind to (e.g., "0.0.0.0" for all interfaces, "127.0.0.1" for localhost)
    /// * `port` - Port number (default: 9876)
    /// * `same_machine` - Set to `true` if viewer runs on same machine (avoids double-buffering)
    ///
    /// # Example
    /// ```rust,ignore
    /// // Headless daemon accessible from remote machines
    /// let sink = RerunSink::new_server("0.0.0.0", 9876, false)?;
    ///
    /// // Local development with embedded viewer
    /// let sink = RerunSink::new_server("127.0.0.1", 9876, true)?;
    /// ```
    pub fn new_server(bind_ip: &str, port: u16, same_machine: bool) -> Result<Self> {
        Self::new_server_with_app_id(APP_ID, bind_ip, port, same_machine)
    }

    /// Create a new Rerun gRPC server with a custom application ID.
    ///
    /// See [`new_server`](Self::new_server) for details.
    pub fn new_server_with_app_id(
        application_id: &str,
        bind_ip: &str,
        port: u16,
        same_machine: bool,
    ) -> Result<Self> {
        // Memory limit: 0 for same-machine to avoid double-buffering,
        // 25% of RAM for remote clients
        let memory_limit = if same_machine {
            MemoryLimit::from_bytes(0)
        } else {
            MemoryLimit::from_fraction_of_total(0.25)
        };

        let server_options = ServerOptions {
            memory_limit,
            ..Default::default()
        };

        let rec = RecordingStreamBuilder::new(application_id).serve_grpc_opts(
            bind_ip,
            port,
            server_options,
        )?;

        tracing::info!(
            "Rerun gRPC server started on {}:{} (connect via rerun+http://{}:{}/proxy)",
            bind_ip,
            port,
            if bind_ip == "0.0.0.0" {
                "HOST_IP"
            } else {
                bind_ip
            },
            port
        );

        Ok(Self {
            rec,
            recording_stream: None,
        })
    }

    /// Create a Rerun gRPC server with full control over options.
    ///
    /// # Arguments
    /// * `application_id` - Application ID (must match blueprints)
    /// * `bind_ip` - IP address to bind to
    /// * `port` - Port number
    /// * `server_options` - Full `ServerOptions` for memory limit, playback behavior, etc.
    pub fn new_server_with_opts(
        application_id: &str,
        bind_ip: &str,
        port: u16,
        server_options: ServerOptions,
    ) -> Result<Self> {
        let rec = RecordingStreamBuilder::new(application_id).serve_grpc_opts(
            bind_ip,
            port,
            server_options,
        )?;

        tracing::info!(
            "Rerun gRPC server started on {}:{} (connect via rerun+http://{}:{}/proxy)",
            bind_ip,
            port,
            if bind_ip == "0.0.0.0" {
                "HOST_IP"
            } else {
                bind_ip
            },
            port
        );

        Ok(Self {
            rec,
            recording_stream: None,
        })
    }

    /// Create a Rerun gRPC server with simultaneous .rrd file recording.
    ///
    /// This creates two parallel streams:
    /// - gRPC server for live viewers (via `serve_grpc_opts()`)
    /// - File sink for .rrd recording (via `FileSink`)
    ///
    /// Both streams receive all logged data, enabling live visualization
    /// and persistent recording simultaneously.
    ///
    /// # Arguments
    /// * `bind_ip` - IP address to bind to (e.g., "0.0.0.0" for all interfaces)
    /// * `port` - Port number (default: 9876)
    /// * `same_machine` - Set to `true` if viewer runs on same machine
    /// * `recording_path` - Optional path for .rrd file recording
    ///
    /// # Example
    /// ```rust,ignore
    /// // Headless daemon with recording
    /// let sink = RerunSink::new_server_with_recording(
    ///     "0.0.0.0",
    ///     9876,
    ///     false,
    ///     Some("experiment_001.rrd"),
    /// )?;
    /// ```
    pub fn new_server_with_recording(
        bind_ip: &str,
        port: u16,
        same_machine: bool,
        recording_path: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        Self::new_server_with_recording_and_app_id(
            APP_ID,
            bind_ip,
            port,
            same_machine,
            recording_path,
        )
    }

    /// Create a Rerun gRPC server with recording and custom application ID.
    ///
    /// See [`new_server_with_recording`](Self::new_server_with_recording) for details.
    pub fn new_server_with_recording_and_app_id(
        application_id: &str,
        bind_ip: &str,
        port: u16,
        same_machine: bool,
        recording_path: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        // Memory limit: 0 for same-machine to avoid double-buffering,
        // 25% of RAM for remote clients
        let memory_limit = if same_machine {
            MemoryLimit::from_bytes(0)
        } else {
            MemoryLimit::from_fraction_of_total(0.25)
        };

        let server_options = ServerOptions {
            memory_limit,
            ..Default::default()
        };

        // Create primary gRPC server stream
        let rec = RecordingStreamBuilder::new(application_id).serve_grpc_opts(
            bind_ip,
            port,
            server_options,
        )?;

        tracing::info!(
            "Rerun gRPC server started on {}:{} (connect via rerun+http://{}:{}/proxy)",
            bind_ip,
            port,
            if bind_ip == "0.0.0.0" {
                "HOST_IP"
            } else {
                bind_ip
            },
            port
        );

        // Create optional file recording stream
        let recording_stream = if let Some(path) = recording_path {
            let path_ref = path.as_ref();
            let file_sink = FileSink::new(path_ref)
                .map_err(|e| anyhow::anyhow!("Failed to create file sink: {}", e))?;

            let file_rec = RecordingStreamBuilder::new(application_id).set_sinks((file_sink,))?; // Single-element tuple syntax

            tracing::info!("Recording to: {}", path_ref.display());
            Some(Arc::new(file_rec))
        } else {
            None
        };

        Ok(Self {
            rec,
            recording_stream,
        })
    }

    /// Get the connection URL for remote viewers.
    ///
    /// Returns the URL that viewers should use to connect to this server.
    pub fn connection_url(bind_ip: &str, port: u16) -> String {
        let display_ip = if bind_ip == "0.0.0.0" {
            "127.0.0.1"
        } else {
            bind_ip
        };
        format!("rerun+http://{}:{}/proxy", display_ip, port)
    }

    /// Load a blueprint from an .rbl file.
    ///
    /// The blueprint's application ID must match the recording's application ID.
    /// Generate blueprints using `crates/daq-server/blueprints/generate_blueprints.py`.
    ///
    /// # Example
    /// ```rust,ignore
    /// let sink = RerunSink::new()?;
    /// sink.load_blueprint("crates/daq-server/blueprints/daq_default.rbl")?;
    /// ```
    pub fn load_blueprint(&self, path: impl AsRef<Path>) -> Result<()> {
        self.rec.log_file_from_path(
            path, None, // No entity path prefix
            true, // Static (blueprint doesn't change over time)
        )?;
        Ok(())
    }

    /// Load a blueprint only if the file exists.
    ///
    /// Returns `Ok(true)` if the blueprint was loaded, `Ok(false)` if the path
    /// does not exist, and `Err` if loading failed.
    pub fn load_blueprint_if_exists(&self, path: impl AsRef<Path>) -> Result<bool> {
        let path_ref = path.as_ref();
        if !path_ref.exists() {
            return Ok(false);
        }
        self.load_blueprint(path_ref)?;
        Ok(true)
    }

    /// Subscribe to a broadcast channel and log all received measurements.
    pub fn monitor_broadcast(&self, mut rx: tokio::sync::broadcast::Receiver<Measurement>) {
        let rec = self.rec.clone();
        let recording = self.recording_stream.clone();
        tokio::spawn(async move {
            while let Ok(meas) = rx.recv().await {
                Self::log_measurement(&rec, recording.as_deref(), meas);
            }
        });
    }

    /// Log a measurement to a single RecordingStream.
    fn log_measurement_to_stream(rec: &RecordingStream, meas: &Measurement) {
        let name = match meas {
            Measurement::Scalar { name, .. } => name,
            Measurement::Vector { name, .. } => name,
            Measurement::Image { name, .. } => name,
            Measurement::Spectrum { name, .. } => name,
        };

        let entity_path = format!("device/{}", name);

        // Extract timestamp
        let ts = match meas {
            Measurement::Scalar { timestamp, .. } => timestamp,
            Measurement::Vector { timestamp, .. } => timestamp,
            Measurement::Image { timestamp, .. } => timestamp,
            Measurement::Spectrum { timestamp, .. } => timestamp,
        };

        rec.set_time(
            "stable_time",
            rerun::TimeCell::from_timestamp_nanos_since_epoch(
                ts.timestamp_nanos_opt().unwrap_or(0),
            ),
        );

        match meas {
            Measurement::Scalar { value, .. } => {
                let _ = rec.log(entity_path, &Scalars::new([*value]));
            }
            Measurement::Image {
                width,
                height,
                buffer,
                metadata,
                ..
            } => {
                let shape = vec![*height as u64, *width as u64];

                // Log tensor with dimension names for better visualization
                match buffer {
                    PixelBuffer::U8(data) => {
                        let tensor_data = rerun::TensorData::new(
                            shape,
                            rerun::TensorBuffer::U8(data.clone().into()),
                        );
                        let tensor = Tensor::new(tensor_data).with_dim_names(["height", "width"]);
                        let _ = rec.log(entity_path.clone(), &tensor);
                    }
                    PixelBuffer::U16(data) => {
                        let tensor_data = rerun::TensorData::new(
                            shape,
                            rerun::TensorBuffer::U16(data.clone().into()),
                        );
                        let tensor = Tensor::new(tensor_data).with_dim_names(["height", "width"]);
                        let _ = rec.log(entity_path.clone(), &tensor);
                    }
                    _ => {}
                }

                // Log image metadata as separate scalars for time-series visualization
                Self::log_image_metadata(rec, &entity_path, metadata);
            }
            _ => {}
        }
    }

    /// Log image metadata as separate scalar entities.
    ///
    /// This enables time-series visualization of camera parameters like
    /// exposure, gain, and temperature alongside the image data.
    fn log_image_metadata(rec: &RecordingStream, base_path: &str, metadata: &ImageMetadata) {
        let meta_path = format!("{}/metadata", base_path);

        // Log exposure time if available
        if let Some(exposure_ms) = metadata.exposure_ms {
            let _ = rec.log(
                format!("{}/exposure_ms", meta_path),
                &Scalars::new([exposure_ms]),
            );
        }

        // Log gain if available
        if let Some(gain) = metadata.gain {
            let _ = rec.log(format!("{}/gain", meta_path), &Scalars::new([gain]));
        }

        // Log sensor temperature if available
        if let Some(temp_c) = metadata.temperature_c {
            let _ = rec.log(
                format!("{}/temperature_c", meta_path),
                &Scalars::new([temp_c]),
            );
        }

        // Log readout time if available
        if let Some(readout_ms) = metadata.readout_ms {
            let _ = rec.log(
                format!("{}/readout_ms", meta_path),
                &Scalars::new([readout_ms]),
            );
        }

        // Log binning as separate scalars if available
        if let Some((bin_x, bin_y)) = metadata.binning {
            let _ = rec.log(
                format!("{}/binning_x", meta_path),
                &Scalars::new([bin_x as f64]),
            );
            let _ = rec.log(
                format!("{}/binning_y", meta_path),
                &Scalars::new([bin_y as f64]),
            );
        }
    }

    /// Log a measurement to both the primary stream and optional recording stream.
    fn log_measurement(
        rec: &RecordingStream,
        recording: Option<&RecordingStream>,
        meas: Measurement,
    ) {
        // Log to primary (visualization) stream
        Self::log_measurement_to_stream(rec, &meas);

        // Log to recording stream if present
        if let Some(file_rec) = recording {
            Self::log_measurement_to_stream(file_rec, &meas);
        }
    }

    /// Check if recording is enabled.
    pub fn is_recording(&self) -> bool {
        self.recording_stream.is_some()
    }

    /// Get the recording stream reference (for advanced use cases).
    pub fn recording_stream(&self) -> Option<&RecordingStream> {
        self.recording_stream.as_ref().map(|arc| arc.as_ref())
    }

    /// Log a heartbeat timestamp to `/system/heartbeat`.
    ///
    /// This allows remote viewers to detect if the server is alive and data is flowing.
    /// The heartbeat logs the current Unix timestamp as a scalar.
    pub fn log_heartbeat(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        self.rec.set_time(
            "stable_time",
            rerun::TimeCell::from_timestamp_nanos_since_epoch(now.as_nanos() as i64),
        );

        let _ = self
            .rec
            .log("/system/heartbeat", &Scalars::new([now.as_secs_f64()]));

        // Also log to recording stream if present
        if let Some(file_rec) = &self.recording_stream {
            file_rec.set_time(
                "stable_time",
                rerun::TimeCell::from_timestamp_nanos_since_epoch(now.as_nanos() as i64),
            );
            let _ = file_rec.log("/system/heartbeat", &Scalars::new([now.as_secs_f64()]));
        }
    }

    /// Start a background task that logs heartbeats at the specified interval.
    ///
    /// Returns a `JoinHandle` that can be used to abort the task when no longer needed.
    ///
    /// # Example
    /// ```rust,ignore
    /// let sink = RerunSink::new_server("0.0.0.0", 9876, false)?;
    /// let heartbeat_handle = sink.start_heartbeat_task(Duration::from_secs(5));
    ///
    /// // ... later, when shutting down:
    /// heartbeat_handle.abort();
    /// ```
    pub fn start_heartbeat_task(&self, interval: Duration) -> JoinHandle<()> {
        let rec = self.rec.clone();
        let recording = self.recording_stream.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();

                rec.set_time(
                    "stable_time",
                    rerun::TimeCell::from_timestamp_nanos_since_epoch(now.as_nanos() as i64),
                );
                let _ = rec.log("/system/heartbeat", &Scalars::new([now.as_secs_f64()]));

                if let Some(file_rec) = &recording {
                    file_rec.set_time(
                        "stable_time",
                        rerun::TimeCell::from_timestamp_nanos_since_epoch(now.as_nanos() as i64),
                    );
                    let _ = file_rec.log("/system/heartbeat", &Scalars::new([now.as_secs_f64()]));
                }
            }
        })
    }
}

#[async_trait]
impl MeasurementSink for RerunSink {
    type Input = Measurement;
    type Error = anyhow::Error;

    fn register_input(
        &mut self,
        mut rx: mpsc::Receiver<Self::Input>,
    ) -> Result<JoinHandle<()>, Self::Error> {
        let rec = self.rec.clone();
        let recording = self.recording_stream.clone();

        Ok(tokio::spawn(async move {
            while let Some(meas) = rx.recv().await {
                Self::log_measurement(&rec, recording.as_deref(), meas);
            }
        }))
    }
}
