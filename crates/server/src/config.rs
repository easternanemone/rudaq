use anyhow::{Context, Result};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub grpc: GrpcSettings,
    #[serde(default)]
    pub storage: StorageSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GrpcSettings {
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
    pub auth_enabled: bool,
    pub auth_token: Option<String>,
    pub allowed_origins: Vec<String>,
    pub bind_address: Option<IpAddr>,
}

impl Default for GrpcSettings {
    fn default() -> Self {
        Self {
            tls_cert_path: None,
            tls_key_path: None,
            auth_enabled: false,
            auth_token: None,
            allowed_origins: Vec::new(),
            bind_address: Some(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        }
    }
}

impl GrpcSettings {
    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    pub fn bind_socket(&self, default_port: u16) -> SocketAddr {
        let bind_ip = self
            .bind_address
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        SocketAddr::new(bind_ip, default_port)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageSettings {
    pub ring_buffer_path: PathBuf,
    pub ring_buffer_size_mb: usize,
    pub hdf5_path: PathBuf,
    pub output_directory: PathBuf,
    /// Channel capacity for ring buffer taps (frames buffered per tap)
    pub tap_channel_size: usize,
}

impl Default for StorageSettings {
    fn default() -> Self {
        let (ring_buffer_path, hdf5_path) = if cfg!(target_os = "linux") {
            (
                PathBuf::from("/dev/shm/rust_daq_scan_data.buf"),
                PathBuf::from("/tmp/rust_daq_scan_data.h5"),
            )
        } else {
            (
                PathBuf::from("/tmp/rust_daq_scan_data.buf"),
                PathBuf::from("/tmp/rust_daq_scan_data.h5"),
            )
        };

        Self {
            ring_buffer_path,
            ring_buffer_size_mb: 100,
            hdf5_path,
            output_directory: PathBuf::from("./data"),
            tap_channel_size: 32,
        }
    }
}

impl ServerConfig {
    pub fn load() -> Result<Self> {
        let config_path = PathBuf::from("config/config.v4.toml");
        let mut figment = Figment::from(Serialized::defaults(ServerConfig {
            grpc: GrpcSettings::default(),
            storage: StorageSettings::default(),
        }))
        .merge(Env::prefixed("RUSTDAQ_").split("__"));

        if config_path.exists() {
            figment = figment.merge(Toml::file(&config_path));
        } else {
            eprintln!(
                "⚠️  Config file not found at {} (using defaults/env overrides)",
                config_path.display()
            );
        }

        let config: ServerConfig = figment
            .extract()
            .context("failed to extract ServerConfig from figment (check config/config.v4.toml and RUSTDAQ_ env vars)")?;
        Ok(config)
    }
}
