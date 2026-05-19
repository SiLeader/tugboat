// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod auth;
mod compress;
pub mod pull;
pub mod push;

use oci_distribution::client::{ClientConfig, ClientProtocol};
use oci_distribution::errors::OciDistributionError;
use oci_distribution::{Client, ParseError};
use thiserror::Error;

pub(crate) const METADATA_MEDIA_TYPE: &str = "application/vnd.tugboat.metadata.v1+json";

pub(crate) const MAX_DISK_IMAGE_UNCOMPRESSED_BYTES: u64 = 128 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct VmImageRegistry {
    directory: std::path::PathBuf,
}

impl VmImageRegistry {
    pub fn new(directory: impl AsRef<std::path::Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    fn get_client(&self, registry: &str, insecure: Option<bool>) -> Client {
        Client::new(ClientConfig {
            protocol: if insecure.unwrap_or_else(|| {
                registry.starts_with("localhost") || registry.starts_with("host.docker.internal")
            }) {
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
    X64,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Format {
    Qcow2,
    Raw,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qcow2 => "qcow2",
            Self::Raw => "raw",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Qcow2 => "disk.qcow2",
            Self::Raw => "disk.raw",
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Self::Qcow2 => "application/vnd.tugboat.disk.qcow2.v1+gzip",
            Self::Raw => "application/vnd.tugboat.disk.raw.v1+gzip",
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Format {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "qcow2" => Ok(Self::Qcow2),
            "raw" => Ok(Self::Raw),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("OCI Distribution Error: {0}")]
    Oci(#[from] OciDistributionError),
    #[error("Failed to parse tag: {0}")]
    ParseTag(#[from] ParseError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cannot encode file location")]
    FileLocationEncode,
    #[error("Disk image '{0}' is missing")]
    DiskImageMissing(String),
    #[error("Image '{0}' contains multiple supported disk image layers")]
    MultipleDiskImageLayers(String),
    #[error("Disk image layer exceeds maximum uncompressed size of {limit} bytes")]
    ImageLayerTooLarge { limit: u64 },
}

#[cfg(test)]
mod tests {
    use super::Format;
    use std::str::FromStr;

    #[test]
    fn format_helpers_return_external_values() {
        assert_eq!(Format::Qcow2.as_str(), "qcow2");
        assert_eq!(Format::Qcow2.file_name(), "disk.qcow2");
        assert_eq!(
            Format::Qcow2.media_type(),
            "application/vnd.tugboat.disk.qcow2.v1+gzip"
        );

        assert_eq!(Format::Raw.as_str(), "raw");
        assert_eq!(Format::Raw.file_name(), "disk.raw");
        assert_eq!(
            Format::Raw.media_type(),
            "application/vnd.tugboat.disk.raw.v1+gzip"
        );
    }

    #[test]
    fn format_from_str_accepts_exact_lowercase_values() {
        assert_eq!(Format::from_str("qcow2"), Ok(Format::Qcow2));
        assert_eq!(Format::from_str("raw"), Ok(Format::Raw));
        assert!(Format::from_str("QCOW2").is_err());
        assert!(Format::from_str("Raw").is_err());
        assert!(Format::from_str("vmdk").is_err());
    }
}
