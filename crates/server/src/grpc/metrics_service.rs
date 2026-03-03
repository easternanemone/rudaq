//! Prometheus metrics endpoint for observability (bd-v299)
//!
//! Exposes internal metrics in Prometheus text format for integration with
//! monitoring systems like Grafana.
//!
//! # Metrics Exposed
//!
//! ## Gauges (current values)
//! - `run_engine_active_streams`: Current number of active document streams
//!
//! ## Counters (monotonically increasing)
//! - `run_engine_documents_total`: Total documents converted and streamed
//! - `run_engine_lag_events_total`: Total lag events (client fell behind)
//!
//! # Usage
//!
//! Start the metrics server alongside the gRPC server:
//! ```rust,ignore
//! use daq_server::grpc::metrics_service::{start_metrics_server, DaqMetrics};
//!
//! let metrics = DaqMetrics::new();
//! let handle = start_metrics_server(9091, metrics.clone()).await?;
//! ```
//!
//! Then scrape metrics at `http://localhost:9091/metrics`

use prometheus::{
    Encoder, IntCounter, IntGauge, Registry, TextEncoder, gather,
    register_int_counter_with_registry, register_int_gauge_with_registry,
};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::LazyLock;

/// Global metrics registry for DAQ server
pub static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

/// DAQ server metrics collection
///
/// Thread-safe struct containing all observability metrics.
/// Clone this to share across services.
#[derive(Clone)]
pub struct DaqMetrics {
    /// Current number of active document streams
    pub active_streams: IntGauge,

    /// Total documents converted and sent to clients
    pub documents_total: IntCounter,

    /// Total lag events (clients fell behind broadcast)
    pub lag_events_total: IntCounter,

    /// Total bytes streamed to clients
    pub bytes_streamed: IntCounter,

    /// Current gRPC connections
    pub grpc_connections: IntGauge,

    /// RunEngine state (0=Idle, 1=Running, 2=Paused, 3=Error)
    pub engine_state: IntGauge,
}

impl DaqMetrics {
    /// Create a new metrics collection registered with the global registry
    pub fn new() -> Self {
        Self::with_registry(&REGISTRY)
    }

    /// Create a new metrics collection with a custom registry
    pub fn with_registry(registry: &Registry) -> Self {
        let active_streams = register_int_gauge_with_registry!(
            "run_engine_active_streams",
            "Current number of active document streams",
            registry
        )
        .expect("Failed to create active_streams gauge");

        let documents_total = register_int_counter_with_registry!(
            "run_engine_documents_total",
            "Total documents converted and streamed to clients",
            registry
        )
        .expect("Failed to create documents_total counter");

        let lag_events_total = register_int_counter_with_registry!(
            "run_engine_lag_events_total",
            "Total lag events where clients fell behind broadcast",
            registry
        )
        .expect("Failed to create lag_events_total counter");

        let bytes_streamed = register_int_counter_with_registry!(
            "run_engine_bytes_streamed_total",
            "Total bytes streamed to clients",
            registry
        )
        .expect("Failed to create bytes_streamed counter");

        let grpc_connections = register_int_gauge_with_registry!(
            "daq_server_grpc_connections",
            "Current number of gRPC connections",
            registry
        )
        .expect("Failed to create grpc_connections gauge");

        let engine_state = register_int_gauge_with_registry!(
            "run_engine_state",
            "Current RunEngine state (0=Idle, 1=Running, 2=Paused, 3=Error)",
            registry
        )
        .expect("Failed to create engine_state gauge");

        Self {
            active_streams,
            documents_total,
            lag_events_total,
            bytes_streamed,
            grpc_connections,
            engine_state,
        }
    }

    /// Increment active streams count
    pub fn stream_started(&self) {
        self.active_streams.inc();
    }

    /// Decrement active streams count
    pub fn stream_ended(&self) {
        self.active_streams.dec();
    }

    /// Record a document being sent
    pub fn document_sent(&self) {
        self.documents_total.inc();
    }

    /// Record a lag event
    pub fn lag_occurred(&self) {
        self.lag_events_total.inc();
    }

    /// Set engine state
    pub fn set_engine_state(&self, state: EngineState) {
        self.engine_state.set(state as i64);
    }
}

impl Default for DaqMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Engine state enum for metrics
#[derive(Clone, Copy, Debug)]
#[repr(i64)]
pub enum EngineState {
    Idle = 0,
    Running = 1,
    Paused = 2,
    Error = 3,
}

/// Handle returned by start_metrics_server for cleanup
pub struct MetricsServerHandle {
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

/// Start the Prometheus metrics HTTP server
///
/// Spawns a lightweight HTTP server that exposes metrics at `/metrics` endpoint.
/// Returns a handle that will stop the server when dropped.
///
/// # Arguments
/// * `port` - Port to listen on (default: 9091)
///
/// # Example
/// ```rust,ignore
/// let handle = start_metrics_server(9091).await?;
/// // Server runs in background
/// // Dropping handle stops the server
/// ```
pub async fn start_metrics_server(
    port: u16,
) -> Result<MetricsServerHandle, Box<dyn std::error::Error + Send + Sync>> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Create a simple HTTP service
    let make_service = hyper::service::make_service_fn(|_conn| async {
        Ok::<_, Infallible>(hyper::service::service_fn(handle_metrics_request))
    });

    let server = hyper::Server::bind(&addr)
        .serve(make_service)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });

    tracing::info!(
        port = port,
        "Starting Prometheus metrics server at /metrics"
    );

    tokio::spawn(async move {
        if let Err(e) = server.await {
            tracing::error!("Metrics server error: {}", e);
        }
    });

    Ok(MetricsServerHandle {
        _shutdown_tx: shutdown_tx,
    })
}

/// Handle incoming HTTP requests for metrics
async fn handle_metrics_request(
    req: hyper::Request<hyper::Body>,
) -> Result<hyper::Response<hyper::Body>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&hyper::Method::GET, "/metrics") => {
            // Gather and encode metrics
            let encoder = TextEncoder::new();
            let mut metric_families = REGISTRY.gather();
            metric_families.extend(gather());
            let mut buffer = Vec::new();

            match encoder.encode(&metric_families, &mut buffer) {
                Ok(()) => Ok(hyper::Response::builder()
                    .status(200)
                    .header("Content-Type", encoder.format_type())
                    .body(hyper::Body::from(buffer))
                    .expect("static HTTP response with valid headers")),
                Err(e) => {
                    tracing::error!("Failed to encode metrics: {}", e);
                    Ok(hyper::Response::builder()
                        .status(500)
                        .body(hyper::Body::from(format!(
                            "Failed to encode metrics: {}",
                            e
                        )))
                        .expect("static HTTP 500 response"))
                }
            }
        }
        (&hyper::Method::GET, "/health") => Ok(hyper::Response::builder()
            .status(200)
            .body(hyper::Body::from("OK"))
            .expect("static HTTP 200 response")),
        _ => Ok(hyper::Response::builder()
            .status(404)
            .body(hyper::Body::from("Not Found"))
            .expect("static HTTP 404 response")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let registry = Registry::new();
        let metrics = DaqMetrics::with_registry(&registry);

        // Initial values should be 0
        assert_eq!(metrics.active_streams.get(), 0);

        // Test increment/decrement
        metrics.stream_started();
        assert_eq!(metrics.active_streams.get(), 1);

        metrics.stream_started();
        assert_eq!(metrics.active_streams.get(), 2);

        metrics.stream_ended();
        assert_eq!(metrics.active_streams.get(), 1);
    }

    #[test]
    fn test_document_counter() {
        let registry = Registry::new();
        let metrics = DaqMetrics::with_registry(&registry);

        metrics.document_sent();
        metrics.document_sent();
        metrics.document_sent();

        assert_eq!(metrics.documents_total.get(), 3);
    }

    #[test]
    fn test_engine_state() {
        let registry = Registry::new();
        let metrics = DaqMetrics::with_registry(&registry);

        metrics.set_engine_state(EngineState::Idle);
        assert_eq!(metrics.engine_state.get(), 0);

        metrics.set_engine_state(EngineState::Running);
        assert_eq!(metrics.engine_state.get(), 1);

        metrics.set_engine_state(EngineState::Paused);
        assert_eq!(metrics.engine_state.get(), 2);
    }
}
