use serde::Deserialize;
use std::fs::File;
use tugboat_resources::manifests::core::v1::CpuSpec;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct VmConfig {
    pub image: String,
    pub cpu: CpuSpec,
    pub memory: u64,
    pub id: String,
}

impl VmConfig {
    pub(super) fn load_or_panic(path: String) -> Self {
        if path == "-" {
            serde_json::from_reader(std::io::stdin())
        } else {
            serde_json::from_reader(File::open(path).expect("Cannot open config file"))
        }
        .expect("Failed to parse VM config file as TOML")
    }
}
