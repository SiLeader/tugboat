use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct AgentConfig {
    pub runtime: RuntimeConfig,
    pub apiserver: ApiserverConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RuntimeConfig {
    pub executable: String,
    pub config_file: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
}

impl AgentConfig {
    pub(crate) fn load_or_panic(path: impl AsRef<std::path::Path>) -> Self {
        let content = std::fs::read_to_string(path).expect("Failed to read config file");
        toml::from_str(&content).expect("Failed to parse config file as TOML")
    }
}
