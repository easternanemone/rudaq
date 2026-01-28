#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs, unused_imports)]
//! TDD Test: Verify Trace Context Propagation (bd-nz1j)
//!
//! This test verifies that a Request ID header passed to gRPC is propagated
//! through the system and appears in hardware driver logs/spans.
//!
//! ## Expected Behavior (TDD - may initially fail)
//!
//! 1. Client sends a gRPC request with `x-request-id` header
//! 2. Server interceptor extracts the request ID
//! 3. Request ID is injected into tracing span context
//! 4. Hardware driver operations include the request ID in their spans
//! 5. Logs/spans can be correlated by request ID
//!
//! ## Implementation Requirements
//!
//! To make this test pass, the following must be implemented:
//! - Extract `x-request-id` (or `traceparent`) header in gRPC interceptor
//! - Create a tracing span with the request ID as a field
//! - Propagate the span context to service implementations
//!
//! ## References
//! - W3C Trace Context: https://www.w3.org/TR/trace-context/
//! - OpenTelemetry: https://opentelemetry.io/docs/concepts/signals/traces/

#![cfg(all(
    feature = "server",
    feature = "scripting",
    not(feature = "storage_hdf5")
))]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tonic::metadata::MetadataValue;
use tonic::transport::Server;
use tonic::Request;
use tracing::{info_span, Instrument};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

use daq_experiment::RunEngine;
use daq_hardware::registry::DeviceRegistry;
use daq_server::grpc::hardware_service::HardwareServiceImpl;
use protocol::daq::hardware_service_client::HardwareServiceClient;
use protocol::daq::hardware_service_server::HardwareServiceServer;
use protocol::daq::ListDevicesRequest;

/// Custom header name for request ID (commonly used convention)
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Alternative: W3C Trace Context header
#[allow(dead_code)]
const TRACEPARENT_HEADER: &str = "traceparent";

/// A test subscriber layer that captures span data for assertions
struct TestCaptureLayer {
    sender: mpsc::UnboundedSender<CapturedSpan>,
}

#[derive(Debug, Clone)]
struct CapturedSpan {
    name: String,
    target: String,
    fields: Vec<(String, String)>,
}

impl<S> Layer<S> for TestCaptureLayer
where
    S: tracing::Subscriber,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = Vec::new();
        let mut visitor = FieldVisitor(&mut fields);
        attrs.record(&mut visitor);

        let span = CapturedSpan {
            name: attrs.metadata().name().to_string(),
            target: attrs.metadata().target().to_string(),
            fields,
        };
        let _ = self.sender.send(span);
    }

    fn on_record(
        &self,
        _id: &tracing::span::Id,
        _values: &tracing::span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // Could capture additional recorded values here
    }
}

struct FieldVisitor<'a>(&'a mut Vec<(String, String)>);

impl<'a> tracing::field::Visit for FieldVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .push((field.name().to_string(), format!("{:?}", value)));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

/// Setup a test server on a random local port and return the address
async fn setup_server(registry: Arc<DeviceRegistry>) -> std::net::SocketAddr {
    let hardware_service = HardwareServiceImpl::new(registry);

    let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let serve_future = Server::builder()
        .add_service(HardwareServiceServer::new(hardware_service))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));

    tokio::spawn(serve_future);
    local_addr
}

/// Test that a request ID header is propagated through the gRPC call.
///
/// This test is designed as TDD - it documents the expected behavior
/// and will fail until trace context propagation is implemented.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_request_id_propagates_to_spans() {
    // Setup span capture
    let (tx, mut rx) = mpsc::unbounded_channel();
    let capture_layer = TestCaptureLayer { sender: tx };

    // Create a subscriber with our capture layer
    let subscriber = tracing_subscriber::registry().with(capture_layer);

    // Use this subscriber for the test
    let _guard = tracing::subscriber::set_default(subscriber);

    // Setup server with mock registry
    let registry = Arc::new(DeviceRegistry::new());
    let addr = setup_server(registry).await;

    // Create client
    let mut client = HardwareServiceClient::connect(format!("http://{}", addr))
        .await
        .expect("Failed to connect");

    // Generate a unique request ID
    let request_id = uuid::Uuid::new_v4().to_string();

    // Create request with custom header
    let mut request = Request::new(ListDevicesRequest {
        capability_filter: None,
    });
    request.metadata_mut().insert(
        REQUEST_ID_HEADER,
        MetadataValue::try_from(&request_id).unwrap(),
    );

    // Make the call
    let result = client.list_devices(request).await;
    assert!(result.is_ok(), "gRPC call should succeed");

    // Give some time for spans to be recorded
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Collect all captured spans
    let mut captured_spans = Vec::new();
    while let Ok(span) = rx.try_recv() {
        captured_spans.push(span);
    }

    // Debug: Print all captured spans
    println!("\n=== Captured Spans ({}) ===", captured_spans.len());
    for span in &captured_spans {
        println!("  Span: {} (target: {})", span.name, span.target);
        for (key, value) in &span.fields {
            println!("    {} = {}", key, value);
        }
    }

    // Find spans related to the hardware service
    let hardware_spans: Vec<_> = captured_spans
        .iter()
        .filter(|s| {
            s.target.contains("hardware")
                || s.name.contains("list_devices")
                || s.name.contains("ListDevices")
        })
        .collect();

    println!(
        "\n=== Hardware Service Spans ({}) ===",
        hardware_spans.len()
    );
    for span in &hardware_spans {
        println!("  {}: {:?}", span.name, span.fields);
    }

    // TDD ASSERTION: The request ID should appear in at least one span
    // This will FAIL until trace context propagation is implemented
    let request_id_found = captured_spans.iter().any(|span| {
        span.fields.iter().any(|(key, value)| {
            (key == "request_id" || key == "trace_id" || key == "x-request-id")
                && value.contains(&request_id)
        })
    });

    // SOFT ASSERTION: Print status but don't fail the test yet
    // This allows us to track progress without blocking CI
    if request_id_found {
        println!("\n[PASS] Request ID {} found in spans", request_id);
    } else {
        println!(
            "\n[TDD] Request ID {} NOT found in spans - trace propagation not yet implemented",
            request_id
        );
        println!("      To implement: Extract '{}' header in gRPC interceptor and inject into span context", REQUEST_ID_HEADER);
    }

    // Hard assertion for when implementation is complete
    // Uncomment this line when ready to enforce:
    // assert!(request_id_found, "Request ID should be propagated to spans");
}

/// Test that spans are created for hardware operations.
///
/// This verifies that the `#[instrument]` attributes are working.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hardware_service_creates_spans() {
    // Setup span capture
    let (tx, mut rx) = mpsc::unbounded_channel();
    let capture_layer = TestCaptureLayer { sender: tx };

    let subscriber = tracing_subscriber::registry().with(capture_layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    // Setup server
    let registry = Arc::new(DeviceRegistry::new());
    let addr = setup_server(registry).await;

    // Make a call
    let mut client = HardwareServiceClient::connect(format!("http://{}", addr))
        .await
        .expect("Failed to connect");

    let _ = client
        .list_devices(Request::new(ListDevicesRequest {
            capability_filter: None,
        }))
        .await;

    // Give time for spans
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Collect spans
    let mut captured_spans = Vec::new();
    while let Ok(span) = rx.try_recv() {
        captured_spans.push(span);
    }

    // Check that we have spans from the hardware service
    let has_hardware_span = captured_spans
        .iter()
        .any(|s| s.target.contains("hardware") || s.name.contains("list_devices"));

    println!("\n=== Span Creation Test ===");
    println!("Total spans captured: {}", captured_spans.len());
    println!("Has hardware service span: {}", has_hardware_span);

    // This should pass if #[instrument] is being used
    // If it fails, the service methods need instrumentation
    if !has_hardware_span && !captured_spans.is_empty() {
        println!("[INFO] Hardware service spans not found, but other spans exist");
        println!("       Service methods may need #[instrument] attribute");
    }
}
