use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ControllerManagerConfig {
    pub apiserver: ApiserverConfig,
    #[serde(default)]
    pub csi: CsiConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct CsiConfig {
    #[serde(default)]
    pub provisioners: HashMap<String, ProvisionerConfig>,
    #[serde(default = "default_requeue_interval_seconds")]
    pub requeue_interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProvisionerConfig {
    pub socket_path: String,
}

fn default_requeue_interval_seconds() -> u64 {
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
}
