//! Authentication, TLS, and CORS configuration helpers for the gRPC server.
//!
//! Extracted from `server/mod.rs` (C1 step 2) to keep the parent module focused
//! on the runtime `DaqServer` implementation. All public items are scoped to
//! `pub(super)` because they are only consumed by sibling modules
//! (`startup`, `daq_server`) and the test module.

use http::{HeaderValue, header::HeaderName};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use tonic::transport::{Identity, ServerTlsConfig};
use tonic::{Request, Status};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::config::GrpcSettings;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct JwtClaims {
    exp: Option<usize>,
    iss: Option<String>,
    aud: Option<String>,
    sub: Option<String>,
}

pub(super) fn build_tls_config(
    settings: &GrpcSettings,
) -> Result<Option<ServerTlsConfig>, Box<dyn std::error::Error>> {
    let (cert_path, key_path) = match (&settings.tls_cert_path, &settings.tls_key_path) {
        (Some(cert), Some(key)) => (cert, key),
        (None, None) => return Ok(None),
        _ => {
            return Err("gRPC TLS requires both grpc.tls_cert_path and grpc.tls_key_path".into());
        }
    };

    let cert = std::fs::read(cert_path)
        .map_err(|e| format!("Failed to read TLS cert {}: {}", cert_path.display(), e))?;
    let key = std::fs::read(key_path)
        .map_err(|e| format!("Failed to read TLS key {}: {}", key_path.display(), e))?;
    let identity = Identity::from_pem(cert, key);
    Ok(Some(ServerTlsConfig::new().identity(identity)))
}

pub(super) fn build_cors_layer(
    settings: &GrpcSettings,
) -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let mut cors = CorsLayer::new()
        .allow_headers(Any)
        .allow_methods(Any)
        .expose_headers([
            HeaderName::from_static("grpc-status"),
            HeaderName::from_static("grpc-message"),
            HeaderName::from_static("grpc-status-details-bin"),
            HeaderName::from_static("grpc-encoding"),
            HeaderName::from_static("grpc-accept-encoding"),
            HeaderName::from_static("x-grpc-web"),
            HeaderName::from_static("content-type"),
        ]);

    if settings.allowed_origins.iter().any(|o| o == "*") {
        cors = cors.allow_origin(AllowOrigin::any());
    } else if settings.allowed_origins.is_empty() {
        eprintln!("⚠️  grpc.allowed_origins is empty; gRPC-web requests will be blocked by CORS");
        cors = cors.allow_origin(AllowOrigin::list(Vec::<HeaderValue>::new()));
    } else {
        let origins = settings
            .allowed_origins
            .iter()
            .map(|origin| HeaderValue::from_str(origin))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Invalid origin in grpc.allowed_origins: {e}"))?;
        cors = cors.allow_origin(AllowOrigin::list(origins));
    }

    Ok(cors)
}

#[allow(clippy::result_large_err)] // tonic::Status (176 bytes) is the standard gRPC error type
pub(super) fn validate_auth(settings: &GrpcSettings, request: &Request<()>) -> Result<(), Status> {
    if !settings.auth_enabled {
        return Ok(());
    }

    let expected = settings.auth_token().ok_or_else(|| {
        Status::unauthenticated("auth enabled but grpc.auth_token is not configured")
    })?;

    let metadata = request.metadata();
    let header_token = metadata
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token);

    let api_key_header = metadata
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());

    let candidate = header_token.or(api_key_header);
    let Some(token) = candidate else {
        return Err(Status::unauthenticated("missing authorization token"));
    };

    if token == expected {
        return Ok(());
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let decoding_key = DecodingKey::from_secret(expected.as_bytes());
    decode::<JwtClaims>(token, &decoding_key, &validation)
        .map(|_| ())
        .map_err(|_| Status::unauthenticated("invalid authentication token"))
}

pub(super) fn extract_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    let mut parts = trimmed.splitn(2, ' ');
    let scheme = parts.next()?;
    let token = parts.next();
    if token.is_none() {
        return Some(trimmed);
    }
    if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("apikey") {
        return token.map(str::trim);
    }
    Some(trimmed)
}
