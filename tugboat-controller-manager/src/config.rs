use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tugboat_client::{ClientAuth, ClientTlsConfig};
use tugboat_csi_operator::CsiTimeouts;
use tugboat_runtime_common::config::{ConfigLoadError, load_component_toml_config};

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
    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self, ConfigLoadError> {
        load_component_toml_config("tugboat-controller-manager", path)
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

#[cfg(test)]
mod tests {
    use super::ControllerManagerConfig;

    #[test]
    fn sample_config_deserializes() {
        let config = ControllerManagerConfig::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sample-configs/controller-manager/config.toml"
        ))
        .unwrap();

        assert_eq!(config.apiserver.url, "https://apiserver:8443");
        assert_eq!(config.csi.requeue_interval_seconds, 30);
        assert!(config.csi.provisioners.contains_key("hostpath.csi.k8s.io"));
    }

    #[test]
    fn installer_config_template_deserializes_after_rendering() {
        let template = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../installer/systemd/configs/controller-manager.config.toml.tpl"
        ));
        let rendered = template
            .replace("${APISERVER_URL}", "https://apiserver:8443")
            .replace(
                "${APISERVER_CLIENT_TLS_CONFIG}",
                "[apiserver.tls]\nca_cert_path = \"/etc/tugboat/pki/ca.crt\"",
            )
            .replace(
                "${CONTROLLER_MANAGER_APISERVER_AUTH_CONFIG}",
                "[apiserver.auth]\ntype = \"service-account\"\ntoken_path = \"/var/run/secrets/tugboat.cloud/serviceaccount/token\"",
            );

        let config: ControllerManagerConfig = toml::from_str(&rendered).unwrap();

        assert_eq!(config.apiserver.url, "https://apiserver:8443");
        assert_eq!(config.network.requeue_interval_seconds, 30);
    }
}
