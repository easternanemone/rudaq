//! Continuous streaming acquisition module.
//!
//! This module provides high-performance continuous data acquisition with:
//!
//! - Circular buffer management with overflow detection
//! - Backpressure handling for slow consumers
//! - Multi-sink streaming via async channels
//! - Memory-efficient operation
//! - Clean stop on request
//!
//! # Architecture
//!
//! The continuous streaming system uses a producer-consumer pattern:
//!
//! 1. Hardware DMA fills the Comedi buffer
//! 2. Reader thread copies data to application circular buffer
//! 3. Consumer(s) receive data via async channels
//!
//! ```text
//!                   ┌──────────────────┐
//!                   │  Hardware DMA    │
//!                   └────────┬─────────┘
//!                            │
//!                   ┌────────▼─────────┐
//!                   │  Comedi Buffer   │
//!                   └────────┬─────────┘
//!                            │
//!               ┌────────────▼────────────┐
//!               │  ContinuousStream       │
//!               │  (Circular Buffer +     │
//!               │   Backpressure)         │
//!               └─────────┬───────────────┘
//!                         │
//!       ┌─────────────────┼─────────────────┐
//!       │                 │                 │
//!       ▼                 ▼                 ▼
//!   ┌───────┐        ┌────────┐       ┌──────────┐
//!   │ Sink1 │        │ Sink2  │       │  Sink3   │
//!   │ (HDF5)│        │(Display)│      │(Analysis)│
//!   └───────┘        └────────┘       └──────────┘
//! ```
//!
//! # Example
//!
//! ```no_run
//! use driver_comedi::{ComediDevice, ContinuousStream, StreamConfig};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let device = ComediDevice::open("/dev/comedi0")?;
//!
//! let config = StreamConfig::builder()
//!     .channels(&[0, 1, 2, 3])
//!     .sample_rate(100000.0)  // 100 kS/s per channel
//!     .build()?;
//!
//! let mut stream = ContinuousStream::new(&device, config)?;
//!
//! // Add multiple sinks
//! let mut display_rx = stream.add_sink("display", 1000)?;
//! let mut storage_rx = stream.add_sink("storage", 10000)?;
//!
//! stream.start()?;
//!
//! // Process data from sinks
//! while let Some(batch) = display_rx.recv().await {
//!     // Update display
//! }
//!
//! stream.stop()?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use crate::device::ComediDevice;
use crate::error::{ComediError, Result};
use crate::streaming::{StreamAcquisition, StreamConfig, StreamStats};

/// A batch of samples from continuous acquisition.
#[derive(Debug, Clone)]
pub struct SampleBatch {
    /// Sample data as voltages, organized by scan
    /// [scan0_ch0, scan0_ch1, ..., scan1_ch0, scan1_ch1, ...]
    pub data: Vec<f64>,
    /// Number of channels per scan
    pub n_channels: usize,
    /// Timestamp of first sample in batch
    pub timestamp: Instant,
    /// Sequence number for ordering
    pub sequence: u64,
    /// Whether overflow occurred before this batch
    pub overflow_before: bool,
}

impl SampleBatch {
    /// Get the number of complete scans in this batch.
    pub fn n_scans(&self) -> usize {
        if self.n_channels > 0 {
            self.data.len() / self.n_channels
        } else {
            0
        }
    }

    /// Get data for a specific channel.
    pub fn channel_data(&self, channel: usize) -> Vec<f64> {
        if channel >= self.n_channels {
            return Vec::new();
        }

        self.data
            .iter()
            .enumerate()
            .filter_map(|(i, &v)| {
                if i % self.n_channels == channel {
                    Some(v)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Reshape data into per-channel vectors.
    pub fn deinterleave(&self) -> Vec<Vec<f64>> {
        let mut channels = vec![Vec::with_capacity(self.n_scans()); self.n_channels];

        for (i, &v) in self.data.iter().enumerate() {
            channels[i % self.n_channels].push(v);
        }

        channels
    }
}

/// Receiver handle for a streaming sink.
pub type SinkReceiver = mpsc::Receiver<SampleBatch>;

/// Configuration for a streaming sink.
#[derive(Debug, Clone)]
pub struct SinkConfig {
    /// Name of the sink (for logging/identification)
    pub name: String,
    /// Buffer size in number of batches
    pub buffer_size: usize,
    /// Batch size in scans
    pub batch_size: usize,
    /// Whether to drop data on overflow (vs block)
    pub drop_on_overflow: bool,
}

impl Default for SinkConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            buffer_size: 100,
            batch_size: 1000,
            drop_on_overflow: true,
        }
    }
}

/// Internal sink state.
struct Sink {
    #[allow(dead_code)]
    config: SinkConfig,
    sender: mpsc::Sender<SampleBatch>,
    drops: AtomicU64,
}

/// Statistics for continuous streaming.
#[derive(Debug, Clone, Default)]
pub struct ContinuousStats {
    /// Base streaming stats
    pub stream: StreamStats,
    /// Total batches produced
    pub batches_produced: u64,
    /// Total samples dropped due to slow sinks
    pub samples_dropped: u64,
    /// Number of overflow events
    pub overflow_events: u64,
    /// Per-sink drop counts
    pub sink_drops: HashMap<String, u64>,
    /// Current backpressure level (0.0 - 1.0)
    pub backpressure: f64,
}

/// Continuous streaming acquisition with multi-sink support.
///
/// This provides indefinite-duration streaming with backpressure handling
/// and multiple output sinks.
pub struct ContinuousStream {
    #[allow(dead_code)]
    device: ComediDevice,
    config: StreamConfig,
    acquisition: Arc<StreamAcquisition>,
    sinks: Arc<RwLock<HashMap<String, Sink>>>,
    running: Arc<AtomicBool>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    sequence: Arc<AtomicU64>,
    overflow_count: Arc<AtomicU64>,
    batches_produced: Arc<AtomicU64>,
    samples_dropped: Arc<AtomicU64>,
    start_time: Mutex<Option<Instant>>,
}

impl ContinuousStream {
    /// Create a new continuous streaming acquisition.
    pub fn new(device: &ComediDevice, config: StreamConfig) -> Result<Self> {
        // Create the underlying stream acquisition
        let acquisition = Arc::new(StreamAcquisition::new(device, config.clone())?);

        info!(
            n_channels = config.channels.len(),
            sample_rate = config.sample_rate,
            "Created continuous stream"
        );

        Ok(Self {
            device: device.clone(),
            config,
            acquisition,
            sinks: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(false)),
            reader_thread: Mutex::new(None),
            sequence: Arc::new(AtomicU64::new(0)),
            overflow_count: Arc::new(AtomicU64::new(0)),
            batches_produced: Arc::new(AtomicU64::new(0)),
            samples_dropped: Arc::new(AtomicU64::new(0)),
            start_time: Mutex::new(None),
        })
    }

    /// Add a new sink to receive streaming data.
    ///
    /// Returns a receiver that will receive `SampleBatch` values.
    pub fn add_sink(&self, name: &str, batch_size: usize) -> Result<SinkReceiver> {
        self.add_sink_with_config(SinkConfig {
            name: name.to_string(),
            batch_size,
            ..Default::default()
        })
    }

    /// Add a sink with full configuration.
    pub fn add_sink_with_config(&self, config: SinkConfig) -> Result<SinkReceiver> {
        let (tx, rx) = mpsc::channel(config.buffer_size);

        let sink = Sink {
            config: config.clone(),
            sender: tx,
            drops: AtomicU64::new(0),
        };

        let mut sinks = self.sinks.write();
        if sinks.contains_key(&config.name) {
            return Err(ComediError::InvalidConfig {
                message: format!("Sink '{}' already exists", config.name),
            });
        }

        sinks.insert(config.name.clone(), sink);
        debug!(name = config.name, "Added sink");

        Ok(rx)
    }

    /// Remove a sink by name.
    pub fn remove_sink(&self, name: &str) -> bool {
        self.sinks.write().remove(name).is_some()
    }

    /// Get list of sink names.
    pub fn sink_names(&self) -> Vec<String> {
        self.sinks.read().keys().cloned().collect()
    }

    /// Start continuous acquisition.
    pub fn start(&self) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(ComediError::DeviceBusy {
                path: "continuous stream".to_string(),
            });
        }

        // Start the underlying acquisition
        self.acquisition.start()?;

        self.running.store(true, Ordering::SeqCst);
        *self.start_time.lock() = Some(Instant::now());

        // Start reader thread
        let running = Arc::clone(&self.running);
        let acquisition = Arc::clone(&self.acquisition);
        let sinks = Arc::clone(&self.sinks);
        let sequence = Arc::clone(&self.sequence);
        let overflow_count = Arc::clone(&self.overflow_count);
        let batches_produced = Arc::clone(&self.batches_produced);
        let samples_dropped = Arc::clone(&self.samples_dropped);
        let n_channels = self.config.channels.len();
        let batch_size = 1000; // Default batch size in scans

        let handle = thread::spawn(move || {
            let mut buffer = Vec::with_capacity(batch_size * n_channels);
            let mut last_overflow = false;

            while running.load(Ordering::SeqCst) {
                // Read available data
                match acquisition.read_available() {
                    Ok(Some(samples)) => {
                        if samples.is_empty() {
                            // No data available, brief sleep
                            thread::sleep(Duration::from_micros(100));
                            continue;
                        }

                        buffer.extend(samples);

                        // Check for overflow via backpressure metrics
                        let stats = acquisition.stats();
                        let bp = &stats.backpressure;
                        let overflow = bp.buffer_fill_ratio > 0.9;
                        if overflow && !last_overflow {
                            overflow_count.fetch_add(1, Ordering::SeqCst);
                            warn!(
                                buffer_fill_pct =
                                    format_args!("{:.1}", bp.buffer_fill_ratio * 100.0),
                                dropped_samples = bp.dropped_samples,
                                overrun_count = bp.overrun_count,
                                samples_per_second = format_args!("{:.0}", bp.samples_per_second),
                                "Continuous stream: buffer overflow detected"
                            );
                        }
                        last_overflow = overflow;

                        // Dispatch complete batches
                        while buffer.len() >= batch_size * n_channels {
                            let batch_data: Vec<f64> =
                                buffer.drain(..batch_size * n_channels).collect();

                            let batch = SampleBatch {
                                data: batch_data,
                                n_channels,
                                timestamp: Instant::now(),
                                sequence: sequence.fetch_add(1, Ordering::SeqCst),
                                overflow_before: overflow,
                            };

                            // Send to all sinks
                            let sinks_guard = sinks.read();
                            for (name, sink) in sinks_guard.iter() {
                                match sink.sender.try_send(batch.clone()) {
                                    Ok(()) => {}
                                    Err(mpsc::error::TrySendError::Full(_)) => {
                                        sink.drops.fetch_add(1, Ordering::SeqCst);
                                        #[allow(clippy::cast_possible_truncation)]
                                        samples_dropped.fetch_add(
                                            (batch_size * n_channels) as u64,
                                            Ordering::SeqCst,
                                        );
                                        trace!(sink = name, "Dropped batch (sink full)");
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        debug!(sink = name, "Sink closed");
                                    }
                                }
                            }

                            batches_produced.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    Ok(None) => {
                        // Acquisition stopped
                        break;
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }

            debug!("Reader thread exiting");
        });

        *self.reader_thread.lock() = Some(handle);

        info!("Started continuous streaming");
        Ok(())
    }

    /// Stop continuous acquisition.
    pub fn stop(&self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        // Signal stop
        self.running.store(false, Ordering::SeqCst);

        // Stop the acquisition
        self.acquisition.stop()?;

        // Wait for reader thread
        if let Some(handle) = self.reader_thread.lock().take() {
            if let Err(e) = handle.join() {
                error!("Reader thread panicked: {:?}", e);
            }
        }

        let stats = self.stats();
        info!(
            scans = stats.stream.scans_acquired,
            batches = stats.batches_produced,
            drops = stats.samples_dropped,
            overflows = stats.overflow_events,
            "Stopped continuous streaming"
        );

        Ok(())
    }

    /// Check if streaming is active.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get current statistics.
    pub fn stats(&self) -> ContinuousStats {
        let stream_stats = self.acquisition.stats();

        let mut sink_drops = HashMap::new();
        for (name, sink) in self.sinks.read().iter() {
            sink_drops.insert(name.clone(), sink.drops.load(Ordering::SeqCst));
        }

        // Calculate backpressure from sink channel fill levels.
        // We take the maximum fill ratio across all sinks: a value of 1.0 means
        // at least one sink is completely full and dropping data.
        let mut max_sink_pressure: f64 = 0.0;
        for (_, sink) in self.sinks.read().iter() {
            // capacity() returns the bounded channel size; max_capacity gives
            // the original bound even after some sends.
            let capacity = sink.sender.max_capacity();
            if capacity > 0 {
                // The number of messages sitting in the channel is
                // (capacity - available permits).
                let queued = capacity - sink.sender.capacity();
                #[allow(clippy::cast_precision_loss)]
                let fill = queued as f64 / capacity as f64;
                if fill > max_sink_pressure {
                    max_sink_pressure = fill;
                }
            }
        }

        // Blend hardware buffer fill with sink pressure — whichever is higher
        // is the dominant bottleneck.
        let backpressure = max_sink_pressure.max(stream_stats.backpressure.buffer_fill_ratio);

        ContinuousStats {
            stream: stream_stats,
            batches_produced: self.batches_produced.load(Ordering::SeqCst),
            samples_dropped: self.samples_dropped.load(Ordering::SeqCst),
            overflow_events: self.overflow_count.load(Ordering::SeqCst),
            sink_drops,
            backpressure,
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &StreamConfig {
        &self.config
    }

    /// Pause acquisition temporarily.
    pub fn pause(&self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.acquisition.stop()?;
        debug!("Paused continuous streaming");
        Ok(())
    }

    /// Resume acquisition after pause.
    pub fn resume(&self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(ComediError::InvalidConfig {
                message: "Stream not started".to_string(),
            });
        }
        self.acquisition.start()?;
        debug!("Resumed continuous streaming");
        Ok(())
    }
}

impl Drop for ContinuousStream {
    fn drop(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            if let Err(e) = self.stop() {
                error!("Error stopping stream on drop: {}", e);
            }
        }
    }
}

/// Builder for ContinuousStream with additional options.
#[derive(Debug)]
pub struct ContinuousStreamBuilder {
    device: ComediDevice,
    config: StreamConfig,
    default_batch_size: usize,
    default_buffer_size: usize,
}

impl ContinuousStreamBuilder {
    /// Create a new builder.
    pub fn new(device: &ComediDevice, config: StreamConfig) -> Self {
        Self {
            device: device.clone(),
            config,
            default_batch_size: 1000,
            default_buffer_size: 100,
        }
    }

    /// Set the default batch size for new sinks.
    pub fn default_batch_size(mut self, size: usize) -> Self {
        self.default_batch_size = size;
        self
    }

    /// Set the default buffer size for new sinks.
    pub fn default_buffer_size(mut self, size: usize) -> Self {
        self.default_buffer_size = size;
        self
    }

    /// Build the ContinuousStream.
    pub fn build(self) -> Result<ContinuousStream> {
        ContinuousStream::new(&self.device, self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_batch_deinterleave() {
        let batch = SampleBatch {
            data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            n_channels: 2,
            timestamp: Instant::now(),
            sequence: 0,
            overflow_before: false,
        };

        let channels = batch.deinterleave();
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0], vec![1.0, 3.0, 5.0]);
        assert_eq!(channels[1], vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn test_sample_batch_channel_data() {
        let batch = SampleBatch {
            data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
            n_channels: 3,
            timestamp: Instant::now(),
            sequence: 0,
            overflow_before: false,
        };

        assert_eq!(batch.channel_data(0), vec![1.0, 4.0, 7.0]);
        assert_eq!(batch.channel_data(1), vec![2.0, 5.0, 8.0]);
        assert_eq!(batch.channel_data(2), vec![3.0, 6.0, 9.0]);
        assert_eq!(batch.channel_data(5), Vec::<f64>::new()); // Out of range
    }

    #[test]
    fn test_sample_batch_n_scans() {
        let batch = SampleBatch {
            data: vec![0.0; 12],
            n_channels: 4,
            timestamp: Instant::now(),
            sequence: 0,
            overflow_before: false,
        };

        assert_eq!(batch.n_scans(), 3);
    }
}
