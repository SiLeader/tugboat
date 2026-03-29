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

use crate::runtime::RuntimeConfig;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tugboat_cni_operator::CniOperatorConfig;

#[derive(Debug, Deserialize)]
pub(crate) struct AgentConfig {
    pub node: NodeConfig,
    pub runtime: RuntimeConfig,
    pub apiserver: ApiserverConfig,
    pub image: ImageConfig,
    pub cni: CniOperatorConfig,
    #[serde(default)]
    pub csi: CsiConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NodeConfig {
    pub name: String,
    #[serde(default = "default_network_probe_interval_seconds")]
    pub network_probe_interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageConfig {
    pub cache_dir: String,
    #[serde(default)]
    pub http_hosts: HashSet<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct CsiConfig {
    #[serde(default = "default_csi_publish_dir")]
    pub publish_dir: String,
    #[serde(default)]
    pub drivers: HashMap<String, String>,
}

fn default_csi_publish_dir() -> String {
    "/var/lib/tugboat-agent/csi".to_string()
}

fn default_network_probe_interval_seconds() -> u64 {
    30
}

impl AgentConfig {
    pub(crate) fn load_or_panic(path: impl AsRef<std::path::Path>) -> Self {
        let content = std::fs::read_to_string(path).expect("Failed to read config file");
        toml::from_str(&content).expect("Failed to parse config file as TOML")
    }
}

impl NodeConfig {
    pub(crate) fn network_probe_interval(&self) -> Duration {
        Duration::from_secs(self.network_probe_interval_seconds)
    }
}
