//! Pipeline traits for data acquisition
//!
//! Defines the core abstractions for the "Mullet Strategy" pipeline:
//! - **Source**: Produces measurements
//! - **Sink**: Consumes measurements
//! - **Processor**: Transforms measurements
//! - **Tee**: Splits stream into Reliable (mpsc) and Lossy (broadcast) paths
//!
//! # Architecture
//!
//! ```text
//! [Source] --> [Tee] --(mpsc)--> [Storage Sink] (Reliable, Backpressure)
//!                |
//!                --(broadcast)--> [Network Sink] (Lossy, Droppable)
//! ```

pub mod processor;
pub mod sink;
pub mod source;
pub mod tee;

// Re-export main types for convenience
pub use processor::MeasurementProcessor;
pub use sink::MeasurementSink;
pub use source::MeasurementSource;
pub use tee::Tee;
