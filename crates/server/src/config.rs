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
    /// Schema version for forward-compatible config parsing.
    /// Unversioned configs default to 1.
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    #[serde(default)]
    pub grpc: GrpcSettings,
    #[serde(default)]
    pub storage: StorageSettings,
}

fn default_config_version() -> u32 {
    1
}

/// Maximum supported config version. Reject configs from newer software.
const MAX_CONFIG_VERSION: u32 = 1;

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
            output_directory: storage::StorageConfig::from_env().data_path().to_path_buf(),
            tap_channel_size: 32,
        }
    }
}

impl ServerConfig {
    pub fn load() -> Result<Self> {
        let config_path = PathBuf::from("config/config.v4.toml");
        let mut figment = Figment::from(Serialized::defaults(ServerConfig {
            config_version: default_config_version(),
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
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values.
    ///
    /// Returns errors for critical issues (invalid version, missing parents).
    /// Logs warnings for non-critical issues (unwritable output dir).
    pub fn validate(&self) -> Result<()> {
        // Version gate: reject invalid or future versions
        if self.config_version < 1 || self.config_version > MAX_CONFIG_VERSION {
            anyhow::bail!(
                "Config version {} is not supported (expected: 1..={})",
                self.config_version,
                MAX_CONFIG_VERSION
            );
        }

        // Ring buffer size must be at least 1 MB
        if self.storage.ring_buffer_size_mb == 0 {
            anyhow::bail!("ring_buffer_size_mb must be at least 1");
        }

        // Ring buffer path: parent directory must exist
        if let Some(parent) = self.storage.ring_buffer_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            anyhow::bail!(
                "Ring buffer parent directory does not exist: {}",
                parent.display()
            );
        }

        // HDF5 path: parent directory must exist
        if let Some(parent) = self.storage.hdf5_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            anyhow::bail!("HDF5 parent directory does not exist: {}", parent.display());
        }

        // Output directory: warn if not writable (non-fatal)
        // Use is_dir() + temp file probe instead of metadata().readonly()
        // which is unreliable on Unix (only checks owner write bit).
        if self.storage.output_directory.exists() {
            if !self.storage.output_directory.is_dir() {
                eprintln!(
                    "⚠️  Output path is not a directory: {}",
                    self.storage.output_directory.display()
                );
            } else {
                let probe = self.storage.output_directory.join(".daq_write_probe");
                match std::fs::File::create(&probe) {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&probe);
                    }
                    Err(_) => {
                        eprintln!(
                            "⚠️  Output directory is not writable: {}",
                            self.storage.output_directory.display()
                        );
                    }
                }
            }
        }

        Ok(())
    }
}
