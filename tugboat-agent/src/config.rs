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
use std::collections::HashSet;
use tugboat_cni_operator::CniOperatorConfig;

#[derive(Debug, Deserialize)]
pub(crate) struct AgentConfig {
    pub node: NodeConfig,
    pub runtime: RuntimeConfig,
    pub apiserver: ApiserverConfig,
    pub image: ImageConfig,
    pub cni: CniOperatorConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NodeConfig {
    pub name: String,
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

impl AgentConfig {
    pub(crate) fn load_or_panic(path: impl AsRef<std::path::Path>) -> Self {
        let content = std::fs::read_to_string(path).expect("Failed to read config file");
        toml::from_str(&content).expect("Failed to parse config file as TOML")
    }
}
