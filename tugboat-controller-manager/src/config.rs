use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tugboat_client::{ClientAuth, ClientTlsConfig};
use tugboat_csi_operator::CsiTimeouts;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ControllerManagerConfig {
    pub apiserver: ApiserverConfig,
    #[serde(default)]
    pub csi: CsiConfig,
    #[serde(default)]
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
    #[serde(default)]
    pub auth: ClientAuth,
    #[serde(default)]
    pub tls: ClientTlsConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct CsiConfig {
    #[serde(default)]
    pub provisioners: HashMap<String, ProvisionerConfig>,
    #[serde(default = "default_requeue_interval_seconds")]
    pub requeue_interval_seconds: u64,
    #[serde(default = "default_csi_socket_connect_timeout_seconds")]
    pub socket_connect_timeout_seconds: u64,
    #[serde(default = "default_csi_rpc_timeout_seconds")]
    pub rpc_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProvisionerConfig {
    pub socket_path: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct NetworkConfig {
    #[serde(default = "default_requeue_interval_seconds")]
    pub requeue_interval_seconds: u64,
}

fn default_requeue_interval_seconds() -> u64 {
    30
}

fn default_csi_socket_connect_timeout_seconds() -> u64 {
    5
}

fn default_csi_rpc_timeout_seconds() -> u64 {
    30
}

impl ControllerManagerConfig {
    pub(crate) fn load_or_panic(path: impl AsRef<std::path::Path>) -> Self {
        let content = std::fs::read_to_string(path).expect("Failed to read config file");
        toml::from_str(&content).expect("Failed to parse config file as TOML")
    }
}

impl CsiConfig {
    pub(crate) fn requeue_interval(&self) -> Duration {
        Duration::from_secs(self.requeue_interval_seconds)
    }

    pub(crate) fn timeouts(&self) -> CsiTimeouts {
        CsiTimeouts {
            socket_connect_timeout: Duration::from_secs(self.socket_connect_timeout_seconds),
            rpc_call_timeout: Duration::from_secs(self.rpc_timeout_seconds),
        }
    }
}

impl NetworkConfig {
    pub(crate) fn requeue_interval(&self) -> Duration {
        Duration::from_secs(self.requeue_interval_seconds)
    }
}
