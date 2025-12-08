use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

/// Default channel capacity for tap consumers (number of frames buffered)
const DEFAULT_TAP_CHANNEL_SIZE: usize = 16;

/// A tap consumer that receives every Nth frame from the ring buffer.
#[derive(Debug)]
pub struct TapConsumer {
    /// Unique identifier for this tap
    pub id: String,

    /// Deliver every nth frame (1 = every frame, 10 = every 10th frame)
    pub nth_frame: usize,

    /// Current frame count for this tap (internal counter)
    frame_count: AtomicU64,

    /// Async channel sender for delivering frames
    /// Uses try_send to avoid blocking on backpressure
    sender: mpsc::Sender<Vec<u8>>,

    /// Number of dropped frames due to backpressure
    dropped_frames: AtomicU64,
}

impl TapConsumer {
    /// Create a new tap consumer
    pub fn new(id: String, nth_frame: usize, sender: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            id,
            nth_frame: nth_frame.max(1), // Ensure at least 1
            frame_count: AtomicU64::new(0),
            sender,
            dropped_frames: AtomicU64::new(0),
        }
    }

    /// Check if this frame should be delivered based on nth_frame setting
    pub fn should_deliver(&self) -> bool {
        let count = self.frame_count.fetch_add(1, Ordering::Relaxed);
        count % self.nth_frame as u64 == 0
    }

    /// Attempt to send a frame without blocking
    /// Returns true if sent successfully, false if dropped due to backpressure
    pub fn try_send_frame(&self, data: Vec<u8>) -> bool {
        match self.sender.try_send(data) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped_frames.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false, // Receiver closed
        }
    }

    /// Get dropped frame count
    pub fn dropped_count(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }
}

/// Registry for managing active data taps
#[derive(Debug, Default)]
pub struct TapRegistry {
    taps: RwLock<HashMap<String, Arc<TapConsumer>>>,
}

impl TapRegistry {
    pub fn new() -> Self {
        Self {
            taps: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new tap
    pub fn register(&self, id: String, nth_frame: usize) -> Result<mpsc::Receiver<Vec<u8>>> {
        let mut taps = self
            .taps
            .write()
            .map_err(|_| anyhow!("Tap registry lock poisoned"))?;

        if taps.contains_key(&id) {
            return Err(anyhow!("Tap with ID '{}' already exists", id));
        }

        let (tx, rx) = mpsc::channel(DEFAULT_TAP_CHANNEL_SIZE);
        let tap = Arc::new(TapConsumer::new(id.clone(), nth_frame, tx));
        taps.insert(id, tap);

        Ok(rx)
    }

    /// Unregister a tap
    pub fn unregister(&self, id: &str) -> Result<bool> {
        let mut taps = self
            .taps
            .write()
            .map_err(|_| anyhow!("Tap registry lock poisoned"))?;
        Ok(taps.remove(id).is_some())
    }

    /// Notify all taps with new data
    pub fn notify_all(&self, data: &[u8]) {
        let taps = match self.taps.read() {
            Ok(guard) => guard,
            Err(_) => return, // Lock poisoned
        };

        if taps.is_empty() {
            return;
        }

        // Clone data once if needed, or per tap?
        // Optimization: only clone if we have at least one consumer that wants it
        // But here we just iterate.

        // Since we need to send owned Vec<u8> to channel, we might need multiple clones
        // if multiple taps want the frame.
        // To avoid cloning for *skipped* frames, we check should_deliver first.

        for tap in taps.values() {
            if tap.should_deliver() {
                // Clone data only when sending
                // TODO: Use Arc<Vec<u8>> or Bytes for zero-copy multicast?
                // For now, Vec<u8> clone is safer/simpler for this step.
                let frame_data = data.to_vec();
                tap.try_send_frame(frame_data);
            }
        }
    }

    /// Get count of active taps
    pub fn count(&self) -> usize {
        self.taps.read().map(|t| t.len()).unwrap_or(0)
    }

    /// List all taps
    pub fn list(&self) -> Vec<(String, usize)> {
        self.taps
            .read()
            .map(|t| t.values().map(|t| (t.id.clone(), t.nth_frame)).collect())
            .unwrap_or_default()
    }
}
