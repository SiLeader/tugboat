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
use std::path::Path;
use std::time::Duration;
use tugboat_client::{ClientAuth, ClientTlsConfig};
use tugboat_cni_operator::CniOperatorConfig;
use tugboat_csi_operator::CsiTimeouts;
use tugboat_runtime_common::config::{ConfigLoadError, load_component_toml_config};

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
    #[serde(default)]
    pub runtime_class: Option<String>,
    #[serde(default = "default_network_probe_interval_seconds")]
    pub network_probe_interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiserverConfig {
    pub url: String,
    #[serde(default)]
    pub auth: ClientAuth,
    #[serde(default)]
    pub tls: ClientTlsConfig,
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
    #[serde(default = "default_csi_socket_connect_timeout_seconds")]
    pub socket_connect_timeout_seconds: u64,
    #[serde(default = "default_csi_rpc_timeout_seconds")]
    pub rpc_timeout_seconds: u64,
}

fn default_csi_publish_dir() -> String {
    "/var/lib/tugboat-agent/csi".to_string()
}

fn default_network_probe_interval_seconds() -> u64 {
    30
}

fn default_csi_socket_connect_timeout_seconds() -> u64 {
    5
}

fn default_csi_rpc_timeout_seconds() -> u64 {
    30
}

impl AgentConfig {
    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self, ConfigLoadError> {
        load_component_toml_config("tugboat-agent", path)
    }
}

impl NodeConfig {
    pub(crate) fn network_probe_interval(&self) -> Duration {
        Duration::from_secs(self.network_probe_interval_seconds)
    }
}

impl CsiConfig {
    pub(crate) fn timeouts(&self) -> CsiTimeouts {
        CsiTimeouts {
            socket_connect_timeout: Duration::from_secs(self.socket_connect_timeout_seconds),
            rpc_call_timeout: Duration::from_secs(self.rpc_timeout_seconds),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AgentConfig;

    #[test]
    fn sample_config_deserializes() {
        let config = AgentConfig::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sample-configs/agent/config.toml"
        ))
        .unwrap();

        assert_eq!(config.node.name, "node1");
        assert_eq!(config.apiserver.url, "https://apiserver:8443");
        assert_eq!(config.csi.publish_dir, "/var/lib/tugboat-agent/csi");
    }

    #[test]
    fn installer_config_template_deserializes_after_rendering() {
        let template = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../installer/systemd/configs/agent.config.toml.tpl"
        ));
        let rendered = template
            .replace("${NODE_NAME}", "node-a")
            .replace("${RUNTIME_BINARY}", "tugboat-qemu-runtime")
            .replace("${RUNTIME_CONFIG_FILE}", "runtime-qemu.config.toml")
            .replace("${APISERVER_URL}", "https://apiserver:8443")
            .replace(
                "${APISERVER_CLIENT_TLS_CONFIG}",
                "[apiserver.tls]\nca_cert_path = \"/etc/tugboat/pki/ca.crt\"",
            )
            .replace(
                "${AGENT_APISERVER_AUTH_CONFIG}",
                "[apiserver.auth]\ntype = \"service-account\"\ntoken_path = \"/var/run/secrets/tugboat.cloud/serviceaccount/token\"",
            );

        let config: AgentConfig = toml::from_str(&rendered).unwrap();

        assert_eq!(config.node.name, "node-a");
        assert_eq!(
            config.runtime.executable,
            "/usr/local/bin/tugboat-qemu-runtime"
        );
        assert_eq!(config.image.cache_dir, "/var/lib/tugboat-agent/images");
    }
}
