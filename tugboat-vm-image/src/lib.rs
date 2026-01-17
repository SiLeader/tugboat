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
    X64,
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
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "pull")]
    #[error("Cannot encode file location")]
    FileLocationEncode,
    #[cfg(feature = "pull")]
    #[error("Disk image '{0}' is missing")]
    DiskImageMissing(String),
}
