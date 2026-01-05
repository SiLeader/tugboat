mod auth;
#[cfg(feature = "pull")]
pub mod pull;
#[cfg(feature = "push")]
pub mod push;

use oci_distribution::client::{ClientConfig, ClientProtocol};
use oci_distribution::errors::OciDistributionError;
use oci_distribution::{Client, ParseError};
use thiserror::Error;

#[derive(Clone)]
pub struct VmImageRegistry {
    #[cfg(feature = "pull")]
    directory: std::path::PathBuf,
}

#[cfg(not(feature = "pull"))]
impl Default for VmImageRegistry {
    fn default() -> Self {
        Self {}
    }
}

impl VmImageRegistry {
    #[cfg(feature = "pull")]
    pub fn new(directory: std::path::PathBuf) -> Self {
        Self { directory }
    }

    fn get_client(&self, registry: &str, insecure: Option<bool>) -> Client {
        Client::new(ClientConfig {
            protocol: if insecure.unwrap_or(registry.starts_with("localhost")) {
                ClientProtocol::Http
            } else {
                ClientProtocol::Https
            },
            ..Default::default()
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Arch {
    X86_64,
}

#[derive(Debug, Copy, Clone)]
pub enum Format {
    Qcow2,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("OCI Distribution Error: {0}")]
    Oci(#[from] OciDistributionError),
    #[error("Failed to parse tag: {0}")]
    ParseTag(#[from] ParseError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[cfg(feature = "pull")]
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(feature = "pull")]
    #[error("Cannot encode file location")]
    FileLocationEncode,
    #[cfg(feature = "pull")]
    #[error("Disk image '{0}' is missing")]
    DiskImageMissing(String),
}
