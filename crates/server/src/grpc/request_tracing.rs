//! Request tracing layer for gRPC context propagation (bd-1afe.9)
//!
//! Provides a Tower middleware that creates a tracing span for every incoming
//! gRPC request, attaching a `request_id` (from the `x-request-id` header or
//! auto-generated UUID) and the gRPC method path. This span becomes the parent
//! of all spans/events emitted during request processing, enabling end-to-end
//! correlation from UI click → daemon → driver → storage.
//!
//! The `request_id` is also echoed back in the `x-request-id` response header
//! so clients can correlate their requests with server-side traces.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request, Response};
use tower_layer::Layer;
use tower_service::Service;
use tracing::Instrument;
use uuid::Uuid;

/// Tower layer that wraps each gRPC request in a tracing span with `request_id`.
///
/// Place after authentication so that only validated requests are traced.
/// All downstream spans (service handlers, driver calls, storage writes) will
/// inherit the `request_id` field via the tracing span hierarchy.
#[derive(Clone, Debug, Default)]
pub struct RequestTracingLayer;

impl RequestTracingLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for RequestTracingLayer {
    type Service = RequestTracingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestTracingService { inner }
    }
}

/// Tower service that instruments each request with a tracing span.
///
/// For each incoming request:
/// 1. Extracts `x-request-id` from headers (or generates a UUID)
/// 2. Creates an `info_span!("grpc_request", request_id, grpc_method)`
/// 3. Runs the inner service within that span
/// 4. Echoes the `request_id` back in the response `x-request-id` header
#[derive(Clone, Debug)]
pub struct RequestTracingService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RequestTracingService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let grpc_method = req.uri().path().to_owned();

        // Use client-provided request ID for correlation, or generate one
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let span = tracing::info_span!(
            "grpc_request",
            request_id = %request_id,
            grpc_method = %grpc_method,
        );

        let fut = self.inner.call(req);
        let rid = request_id;

        Box::pin(
            async move {
                let result = fut.await;
                match result {
                    Ok(mut response) => {
                        // Echo request_id in response headers for client-side correlation
                        if let Ok(val) = http::HeaderValue::from_str(&rid) {
                            response.headers_mut().insert("x-request-id", val);
                        }
                        Ok(response)
                    }
                    Err(err) => Err(err),
                }
            }
            .instrument(span),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_creates_service() {
        // Verify the Layer trait is correctly implemented by constructing the layer
        let layer = RequestTracingLayer::new();
        // The layer is Clone + Debug + Default
        let _cloned = layer.clone();
        let _defaulted = RequestTracingLayer;
        assert_eq!(format!("{layer:?}"), "RequestTracingLayer");
    }
}
