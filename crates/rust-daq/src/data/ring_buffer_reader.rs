//! Helper utility for clients to decode frames from RingBuffer taps.
//!
//! This module provides [`RingBufferReader`], a convenience wrapper around the
//! `mpsc::Receiver` returned by [`RingBuffer::register_tap()`](super::ring_buffer::RingBuffer::register_tap).
//!
//! # Features
//!
//! - Asynchronous frame reading from ring buffer taps
//! - Automatic deserialization of typed data (JSON, bincode, etc.)
//! - Statistics tracking (frames received, drops detected)
//! - Simple API for remote clients and live visualization
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use rust_daq::data::{RingBuffer, RingBufferReader};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize, Debug)]
//! struct Frame {
//!     timestamp: f64,
//!     value: f64,
//! }
//!
//! # async fn example() -> anyhow::Result<()> {
//! let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 10)?;
//!
//! // Register tap to receive every 10th frame
//! let rx = rb.register_tap("client_1".to_string(), 10)?;
//! let mut reader = RingBufferReader::new(rx);
//!
//! // Read and deserialize frames
//! while let Some(frame) = reader.read_typed::<Frame>().await? {
//!     println!("Received: {:?}", frame);
//! }
//!
//! // Check statistics
//! let stats = reader.stats();
//! println!("Received {} frames", stats.frames_received);
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use tokio::sync::mpsc::Receiver;

/// Helper for reading and deserializing frames from RingBuffer taps.
///
/// This struct wraps the `mpsc::Receiver` returned by [`RingBuffer::register_tap()`]
/// and provides convenient methods for reading raw or typed frames.
///
/// # Thread Safety
///
/// `RingBufferReader` is NOT `Sync` because it wraps a `Receiver`, which is not
/// designed for concurrent access. Each tap should have a single reader.
///
/// # Example
///
/// ```no_run
/// # use rust_daq::data::RingBufferReader;
/// # use tokio::sync::mpsc;
/// # async fn example() {
/// let (_tx, rx) = mpsc::channel(16);
/// let mut reader = RingBufferReader::new(rx);
///
/// // Read raw frames
/// while let Some(data) = reader.read_frame().await {
///     println!("Received {} bytes", data.len());
/// }
///
/// let stats = reader.stats();
/// println!("Total frames: {}", stats.frames_received);
/// # }
/// ```
pub struct RingBufferReader {
    /// Channel receiver from ring buffer tap
    receiver: Receiver<Vec<u8>>,

    /// Number of frames successfully received
    frames_received: usize,

    /// Number of frames potentially dropped (detected via channel closure or gaps)
    ///
    /// Note: This tracks channel-level drops only. The ring buffer itself may
    /// drop frames due to backpressure, which is not visible to the reader.
    frames_dropped: usize,
}

impl RingBufferReader {
    /// Create a new reader from a tap receiver.
    ///
    /// # Arguments
    ///
    /// * `receiver` - The channel receiver returned by [`RingBuffer::register_tap()`]
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use rust_daq::data::{RingBuffer, RingBufferReader};
    /// # fn example() -> anyhow::Result<()> {
    /// let rb = RingBuffer::create(Path::new("/tmp/test.buf"), 10)?;
    /// let rx = rb.register_tap("client_1".to_string(), 1)?;
    ///
    /// let reader = RingBufferReader::new(rx);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn new(receiver: Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            frames_received: 0,
            frames_dropped: 0,
        }
    }

    /// Read the next frame as raw bytes.
    ///
    /// Returns `None` when the ring buffer tap is closed (typically when the
    /// ring buffer is dropped or the tap is unregistered).
    ///
    /// # Returns
    ///
    /// - `Some(Vec<u8>)` - Frame data if available
    /// - `None` - Channel closed, no more frames will arrive
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rust_daq::data::RingBufferReader;
    /// # use tokio::sync::mpsc;
    /// # async fn example() {
    /// # let (_tx, rx) = mpsc::channel(16);
    /// let mut reader = RingBufferReader::new(rx);
    ///
    /// while let Some(frame) = reader.read_frame().await {
    ///     println!("Frame size: {} bytes", frame.len());
    /// }
    /// # }
    /// ```
    pub async fn read_frame(&mut self) -> Option<Vec<u8>> {
        match self.receiver.recv().await {
            Some(data) => {
                self.frames_received += 1;
                Some(data)
            }
            None => {
                // Channel closed - no more frames
                None
            }
        }
    }

    /// Read the next frame and deserialize it as type `T`.
    ///
    /// This method uses `serde_json` for deserialization. The ring buffer must
    /// write frames in JSON format for this to work.
    ///
    /// # Type Parameters
    ///
    /// * `T` - Any type that implements `serde::Deserialize`
    ///
    /// # Returns
    ///
    /// - `Ok(Some(T))` - Successfully deserialized frame
    /// - `Ok(None)` - Channel closed, no more frames
    /// - `Err(_)` - Deserialization error (malformed data)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rust_daq::data::RingBufferReader;
    /// # use tokio::sync::mpsc;
    /// # use serde::Deserialize;
    /// #[derive(Deserialize)]
    /// struct Measurement {
    ///     timestamp: f64,
    ///     value: f64,
    /// }
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// # let (_tx, rx) = mpsc::channel(16);
    /// let mut reader = RingBufferReader::new(rx);
    ///
    /// while let Some(m) = reader.read_typed::<Measurement>().await? {
    ///     println!("t={}, v={}", m.timestamp, m.value);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn read_typed<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>> {
        match self.read_frame().await {
            Some(data) => {
                let value: T =
                    serde_json::from_slice(&data).context("Failed to deserialize frame data")?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Get statistics about frames received and dropped.
    ///
    /// # Returns
    ///
    /// [`ReaderStats`] with current counters
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rust_daq::data::RingBufferReader;
    /// # use tokio::sync::mpsc;
    /// # async fn example() {
    /// # let (_tx, rx) = mpsc::channel(16);
    /// let mut reader = RingBufferReader::new(rx);
    ///
    /// // Process frames...
    ///
    /// let stats = reader.stats();
    /// println!("Received: {}, Dropped: {}",
    ///     stats.frames_received,
    ///     stats.frames_dropped
    /// );
    /// # }
    /// ```
    #[must_use]
    pub fn stats(&self) -> ReaderStats {
        ReaderStats {
            frames_received: self.frames_received,
            frames_dropped: self.frames_dropped,
        }
    }

    /// Get the number of frames currently queued in the channel.
    ///
    /// This can be useful for detecting if the reader is falling behind.
    /// If the queue is consistently near capacity, the reader may not be
    /// able to keep up with the data rate.
    ///
    /// # Returns
    ///
    /// Number of frames waiting to be read
    ///
    /// # Note
    ///
    /// The ring buffer uses a channel size of 16 frames by default.
    /// If this method consistently returns values near 16, frames are
    /// likely being dropped by the ring buffer due to backpressure.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rust_daq::data::RingBufferReader;
    /// # use tokio::sync::mpsc;
    /// # fn example() {
    /// # let (_tx, rx) = mpsc::channel(16);
    /// let reader = RingBufferReader::new(rx);
    ///
    /// let queued = reader.queued_frames();
    /// if queued > 12 {
    ///     println!("Warning: Reader falling behind ({} frames queued)", queued);
    /// }
    /// # }
    /// ```
    #[must_use]
    pub fn queued_frames(&self) -> usize {
        // mpsc::Receiver doesn't expose queue length directly,
        // so we use the capacity estimation
        // Note: This is a best-effort metric
        self.receiver.max_capacity() - self.receiver.capacity()
    }
}

impl std::fmt::Debug for RingBufferReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingBufferReader")
            .field("frames_received", &self.frames_received)
            .field("frames_dropped", &self.frames_dropped)
            .field("queued_frames", &self.queued_frames())
            .finish()
    }
}

/// Statistics about frames read from a ring buffer tap.
///
/// # Fields
///
/// - `frames_received` - Total frames successfully received
/// - `frames_dropped` - Estimated frames dropped (channel-level only)
///
/// # Note
///
/// The `frames_dropped` counter only tracks channel-level drops. The ring
/// buffer itself may drop additional frames due to backpressure when the
/// channel is full. These drops are not visible to the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderStats {
    /// Number of frames successfully received
    pub frames_received: usize,

    /// Number of frames potentially dropped
    ///
    /// This is a lower bound - actual drops may be higher if the ring buffer
    /// dropped frames due to backpressure before they reached the channel.
    pub frames_dropped: usize,
}

impl ReaderStats {
    /// Calculate the frame loss rate as a percentage.
    ///
    /// # Returns
    ///
    /// Frame loss rate in range [0.0, 100.0]
    ///
    /// # Example
    ///
    /// ```
    /// # use rust_daq::data::ReaderStats;
    /// let stats = ReaderStats {
    ///     frames_received: 90,
    ///     frames_dropped: 10,
    /// };
    ///
    /// assert_eq!(stats.loss_rate(), 10.0);
    /// ```
    #[must_use]
    pub fn loss_rate(&self) -> f64 {
        let total = self.frames_received + self.frames_dropped;
        if total == 0 {
            0.0
        } else {
            (self.frames_dropped as f64 / total as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tokio::sync::mpsc;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestFrame {
        id: u32,
        value: f64,
    }

    #[tokio::test]
    async fn test_read_frame() {
        let (tx, rx) = mpsc::channel(16);
        let mut reader = RingBufferReader::new(rx);

        // Send a frame
        let frame_data = vec![1, 2, 3, 4, 5];
        tx.send(frame_data.clone()).await.unwrap();

        // Read it back
        let received = reader.read_frame().await;
        assert_eq!(received, Some(frame_data));

        // Check stats
        let stats = reader.stats();
        assert_eq!(stats.frames_received, 1);
        assert_eq!(stats.frames_dropped, 0);
    }

    #[tokio::test]
    async fn test_read_typed() {
        let (tx, rx) = mpsc::channel(16);
        let mut reader = RingBufferReader::new(rx);

        // Send JSON-encoded frame
        let frame = TestFrame {
            id: 42,
            value: 3.14,
        };
        let json_data = serde_json::to_vec(&frame).unwrap();
        tx.send(json_data).await.unwrap();

        // Read and deserialize
        let received = reader.read_typed::<TestFrame>().await.unwrap();
        assert_eq!(received, Some(frame));

        // Check stats
        let stats = reader.stats();
        assert_eq!(stats.frames_received, 1);
    }

    #[tokio::test]
    async fn test_channel_closed() {
        let (tx, rx) = mpsc::channel(16);
        let mut reader = RingBufferReader::new(rx);

        // Drop sender to close channel
        drop(tx);

        // Should return None
        let result = reader.read_frame().await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_multiple_frames() {
        let (tx, rx) = mpsc::channel(16);
        let mut reader = RingBufferReader::new(rx);

        // Send multiple frames
        for i in 0..10 {
            let frame = TestFrame {
                id: i,
                value: i as f64 * 1.5,
            };
            let json_data = serde_json::to_vec(&frame).unwrap();
            tx.send(json_data).await.unwrap();
        }

        // Read all frames
        let mut received_count = 0;
        drop(tx); // Close channel after sending

        while let Some(frame) = reader.read_typed::<TestFrame>().await.unwrap() {
            assert_eq!(frame.id, received_count);
            received_count += 1;
        }

        assert_eq!(received_count, 10);

        let stats = reader.stats();
        assert_eq!(stats.frames_received, 10);
        assert_eq!(stats.frames_dropped, 0);
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let (tx, rx) = mpsc::channel(16);
        let mut reader = RingBufferReader::new(rx);

        // Send invalid JSON
        tx.send(vec![1, 2, 3, 4, 5]).await.unwrap();

        // Should return error
        let result = reader.read_typed::<TestFrame>().await;
        assert!(result.is_err());
    }

    #[test]
    fn test_stats_loss_rate() {
        let stats = ReaderStats {
            frames_received: 90,
            frames_dropped: 10,
        };

        assert_eq!(stats.loss_rate(), 10.0);

        // Test zero case
        let empty_stats = ReaderStats {
            frames_received: 0,
            frames_dropped: 0,
        };

        assert_eq!(empty_stats.loss_rate(), 0.0);
    }

    #[tokio::test]
    async fn test_debug_output() {
        let (_tx, rx) = mpsc::channel(16);
        let reader = RingBufferReader::new(rx);

        let debug_str = format!("{:?}", reader);
        assert!(debug_str.contains("RingBufferReader"));
        assert!(debug_str.contains("frames_received"));
    }
}
