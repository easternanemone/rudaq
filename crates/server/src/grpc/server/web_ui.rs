//! Web UI static-file Tower layer for serving the WASM frontend.
//!
//! Extracted from `server/mod.rs` (C1 step 2). The layer intercepts
//! non-gRPC requests and serves static files from the WASM UI build
//! directory. When `web_ui_path` is `None`, the layer is a transparent
//! pass-through. It sits as the outermost layer so that non-gRPC
//! requests never reach the CORS or auth middleware.
//!
//! All items are scoped to `pub(super)` because they are only consumed
//! by the `build_grpc_server!` macro in the sibling `startup` module.

use tonic::Status;
use tonic::body::BoxBody;

/// Wrapper that maps `io::Error` body errors to `tonic::Status` for BoxBody compatibility.
pub(super) struct IoToStatusBody<B>(B);

impl<B> http_body::Body for IoToStatusBody<B>
where
    B: http_body::Body<Data = prost::bytes::Bytes, Error = std::io::Error> + Unpin,
{
    type Data = prost::bytes::Bytes;
    type Error = Status;

    fn poll_data(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Self::Data, Self::Error>>> {
        std::pin::Pin::new(&mut self.0).poll_data(cx).map(
            |opt: Option<Result<prost::bytes::Bytes, std::io::Error>>| {
                opt.map(|res| res.map_err(|e| Status::internal(format!("file error: {e}"))))
            },
        )
    }

    fn poll_trailers(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<Option<http::HeaderMap>, Self::Error>> {
        std::pin::Pin::new(&mut self.0)
            .poll_trailers(cx)
            .map_err(|e| Status::internal(format!("file error: {e}")))
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.0.size_hint()
    }
}

/// Tower Layer that serves static WASM UI files for non-gRPC requests.
#[derive(Clone)]
pub(super) struct WebUiLayer {
    serve_dir: Option<tower_http::services::ServeDir>,
}

impl WebUiLayer {
    pub(super) fn new(path: Option<&std::path::PathBuf>) -> Self {
        Self {
            serve_dir: path.map(|p| {
                tower_http::services::ServeDir::new(p).append_index_html_on_directories(true)
            }),
        }
    }
}

impl<S> tower_layer::Layer<S> for WebUiLayer {
    type Service = WebUiService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        WebUiService {
            inner,
            serve_dir: self.serve_dir.clone(),
        }
    }
}

/// Tower Service that routes gRPC requests to the inner service and
/// non-gRPC requests to a static file server.
#[derive(Clone)]
pub(super) struct WebUiService<S> {
    inner: S,
    serve_dir: Option<tower_http::services::ServeDir>,
}

impl<S, B> tower_service::Service<http::Request<B>> for WebUiService<S>
where
    S: tower_service::Service<http::Request<B>, Response = http::Response<BoxBody>>
        + Clone
        + Send
        + 'static,
    S::Error: Send,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = http::Response<BoxBody>;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        // No UI path configured — pass everything through to gRPC
        if self.serve_dir.is_none() {
            return Box::pin(self.inner.call(req));
        }

        // Detect gRPC/gRPC-web requests by content-type header
        let is_grpc = req
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("application/grpc"));

        // Also route by gRPC path prefix (package.Service/Method)
        let is_grpc_path = req.uri().path().starts_with("/daq.")
            || req.uri().path().starts_with("/protocol.")
            || req.uri().path().starts_with("/grpc.");

        if is_grpc || is_grpc_path {
            Box::pin(self.inner.call(req))
        } else {
            let mut files = self.serve_dir.clone().unwrap();
            Box::pin(async move {
                match files.call(req).await {
                    Ok(response) => {
                        // Map ServeDir's io::Error body to tonic's Status body
                        let (parts, body) = response.into_parts();
                        Ok(http::Response::from_parts(
                            parts,
                            BoxBody::new(IoToStatusBody(body)),
                        ))
                    }
                    Err(_io_err) => {
                        // ServeDir failed (e.g. permission denied) — return 500
                        Ok(http::Response::builder()
                            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                            .body(BoxBody::default())
                            .unwrap())
                    }
                }
            })
        }
    }
}
