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

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct SchedulerConfig {
    pub apiserver: ApiserverConfig,
    pub scheduler: SchedulerParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SchedulerParams {
    #[serde(default = "default_scheduler_name")]
    pub name: String,
    #[serde(default = "default_lease_duration")]
    pub lease_duration_seconds: u64,
    #[serde(default = "default_renew_interval")]
    pub renew_interval_seconds: u64,
    #[serde(default = "default_scheduling_interval")]
    pub scheduling_interval_seconds: u64,
    #[serde(default)]
    pub plugins: PluginConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginConfig {
    #[serde(default = "default_filter_plugins")]
    pub filter: Vec<String>,
    #[serde(default = "default_score_plugins")]
    pub score: Vec<String>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            filter: default_filter_plugins(),
            score: default_score_plugins(),
        }
    }
}

fn default_scheduler_name() -> String {
    "default-scheduler".to_string()
}

fn default_lease_duration() -> u64 {
    15
}

fn default_renew_interval() -> u64 {
    10
}

fn default_scheduling_interval() -> u64 {
    1
}

fn default_filter_plugins() -> Vec<String> {
    vec![
        "Unschedulable".to_string(),
        "NetworkFit".to_string(),
        "TaintToleration".to_string(),
        "ResourceFit".to_string(),
        "StorageFit".to_string(),
    ]
}

fn default_score_plugins() -> Vec<String> {
    vec!["TaintToleration".to_string(), "LeastAllocated".to_string()]
}

impl SchedulerConfig {
    pub(crate) fn load_or_panic(path: impl AsRef<std::path::Path>) -> Self {
        let content = std::fs::read_to_string(path).expect("Failed to read config file");
        toml::from_str(&content).expect("Failed to parse config file as TOML")
    }
}
