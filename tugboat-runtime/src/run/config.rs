use serde::Deserialize;
use std::path::Path;
use tugboat_resources::manifests::core::v1::CpuSpec;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct VmConfig {
    pub image: String,
    pub cpu: CpuSpec,
    pub memory: u64,
    pub id: String,
}

impl VmConfig {
    pub(super) fn load_or_panic(path: impl AsRef<Path>) -> Self {
        let content = std::fs::read_to_string(path).expect("Failed to read VM config file");
        serde_json::from_str(&content).expect("Failed to parse VM config file as TOML")
    }
}
