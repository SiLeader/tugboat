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
use std::path::Path;
use tugboat_client::{ClientAuth, ClientTlsConfig};
use tugboat_runtime_common::config::{ConfigLoadError, load_component_toml_config};

#[derive(Debug, Deserialize)]
pub(crate) struct SchedulerConfig {
    pub apiserver: ApiserverConfig,
    pub scheduler: SchedulerParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
    #[serde(default)]
    pub auth: ClientAuth,
    #[serde(default)]
    pub tls: ClientTlsConfig,
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
        "RuntimeClassFit".to_string(),
        "ResourceFit".to_string(),
        "StorageFit".to_string(),
        "StorageBindingReady".to_string(),
        "VolumeTopology".to_string(),
        "NodeAffinity".to_string(),
        "ShipAffinity".to_string(),
        "TopologySpread".to_string(),
    ]
}

fn default_score_plugins() -> Vec<String> {
    vec![
        "TaintToleration".to_string(),
        "LeastAllocated".to_string(),
        "NodeAffinity".to_string(),
        "TopologySpread".to_string(),
        "ImageLocality".to_string(),
    ]
}

impl SchedulerConfig {
    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self, ConfigLoadError> {
        load_component_toml_config("tugboat-scheduler", path)
    }
}

#[cfg(test)]
mod tests {
    use super::SchedulerConfig;

    #[test]
    fn sample_config_deserializes() {
        let config = SchedulerConfig::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sample-configs/scheduler/config.toml"
        ))
        .unwrap();

        assert_eq!(config.scheduler.name, "default-scheduler");
        assert_eq!(config.apiserver.url, "https://apiserver:8443");
        assert!(
            config
                .scheduler
                .plugins
                .filter
                .contains(&"NetworkFit".to_string())
        );
    }

    #[test]
    fn installer_config_template_deserializes_after_rendering() {
        let template = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../installer/systemd/configs/scheduler.config.toml.tpl"
        ));
        let rendered = template
            .replace("${APISERVER_URL}", "https://apiserver:8443")
            .replace(
                "${APISERVER_CLIENT_TLS_CONFIG}",
                "[apiserver.tls]\nca_cert_path = \"/etc/tugboat/pki/ca.crt\"",
            )
            .replace(
                "${SCHEDULER_APISERVER_AUTH_CONFIG}",
                "[apiserver.auth]\ntype = \"service-account\"\ntoken_path = \"/var/run/secrets/tugboat.cloud/serviceaccount/token\"",
            );

        let config: SchedulerConfig = toml::from_str(&rendered).unwrap();

        assert_eq!(config.scheduler.name, "default-scheduler");
        assert_eq!(config.apiserver.url, "https://apiserver:8443");
    }
}
