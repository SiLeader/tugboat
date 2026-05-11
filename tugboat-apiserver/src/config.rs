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

use crate::operator::ApiOperator;
use std::collections::HashMap;
use tugboat_resource_store::{EtcdTlsConfig, ResourceStore};
use tugboat_runtime_common::config::{ConfigLoadError, load_component_toml_config};

#[derive(serde::Deserialize)]
pub struct ApiServerConfig {
    http: HttpConfig,
    etcd: EtcdConfig,
    #[serde(default)]
    authentication: AuthenticationConfig,
    #[serde(default)]
    authorization: AuthorizationConfig,
    #[serde(default)]
    audit: AuditConfig,
}

#[derive(serde::Deserialize)]
pub(crate) struct EtcdConfig {
    endpoints: Vec<String>,
    tls: Option<EtcdTlsConfigToml>,
    #[serde(default)]
    allow_insecure_etcd: bool,
}

#[derive(serde::Deserialize)]
pub(crate) struct EtcdTlsConfigToml {
    ca_cert_path: String,
    cert_path: String,
    key_path: String,
    domain_name: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct HttpConfig {
    listen: String,
    tls: Option<TlsConfig>,
    #[serde(default)]
    allow_insecure_http: bool,
}

#[derive(serde::Deserialize)]
pub struct TlsConfig {
    pub(crate) cert_file: String,
    pub(crate) key_file: String,
    pub(crate) client_cert_file: Option<String>,
}

#[derive(Clone, serde::Deserialize)]
pub struct AuthenticationConfig {
    #[serde(default = "default_anonymous_enabled")]
    pub(crate) anonymous_enabled: bool,
    #[serde(default)]
    pub(crate) service_account: ServiceAccountTokenConfig,
}

#[derive(Clone, serde::Deserialize)]
pub(crate) struct ServiceAccountTokenConfig {
    pub(crate) issuer: Option<String>,
    #[serde(default)]
    pub(crate) audiences: Vec<String>,
    pub(crate) signing_key_file: Option<String>,
    pub(crate) signing_key_id: Option<String>,
    #[serde(default)]
    pub(crate) signing_algorithm: ServiceAccountSigningAlgorithm,
    #[serde(default = "default_service_account_token_ttl_seconds")]
    pub(crate) default_token_ttl_seconds: u64,
    #[serde(default = "default_service_account_token_max_ttl_seconds")]
    pub(crate) max_token_ttl_seconds: u64,
    #[serde(default = "default_service_account_token_leeway_seconds")]
    pub(crate) leeway_seconds: u64,
    #[serde(default)]
    pub(crate) additional_verification_keys: HashMap<String, String>,
}

impl Default for ServiceAccountTokenConfig {
    fn default() -> Self {
        Self {
            issuer: None,
            audiences: Vec::new(),
            signing_key_file: None,
            signing_key_id: None,
            signing_algorithm: ServiceAccountSigningAlgorithm::default(),
            default_token_ttl_seconds: default_service_account_token_ttl_seconds(),
            max_token_ttl_seconds: default_service_account_token_max_ttl_seconds(),
            leeway_seconds: default_service_account_token_leeway_seconds(),
            additional_verification_keys: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Default, serde::Deserialize)]
pub(crate) enum ServiceAccountSigningAlgorithm {
    #[default]
    RS256,
    #[serde(rename = "EdDSA", alias = "eddsa")]
    EdDsa,
}

#[derive(Clone, Default, serde::Deserialize)]
pub struct AuthorizationConfig {
    #[serde(default)]
    pub(crate) mode: AuthorizationMode,
}

#[derive(Clone, Default, serde::Deserialize)]
pub enum AuthorizationMode {
    AlwaysAllow,
    #[default]
    #[serde(rename = "RBAC", alias = "Rbac", alias = "rbac")]
    Rbac,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AuditConfig {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default = "default_audit_log_path")]
    pub(crate) log_path: String,
    // Size-based rotation is not yet wired; the appender rotates daily.
    #[allow(dead_code)]
    #[serde(default = "default_audit_max_size_mb")]
    pub(crate) max_size_mb: u64,
    #[serde(default = "default_audit_max_backups")]
    pub(crate) max_backups: usize,
    // Reserved for future use; max_backups acts as a daily-file cap today.
    #[allow(dead_code)]
    #[serde(default = "default_audit_max_age_days")]
    pub(crate) max_age_days: u64,
    #[serde(default = "default_audit_channel_capacity")]
    pub(crate) channel_capacity: usize,
    #[serde(default = "default_audit_max_request_body_bytes")]
    pub(crate) max_request_body_bytes: usize,
    #[serde(default = "default_audit_max_response_body_bytes")]
    pub(crate) max_response_body_bytes: usize,
    #[serde(default, rename = "rules")]
    pub(crate) rules: Vec<AuditRule>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AuditRule {
    pub(crate) level: AuditLevel,
    #[serde(default)]
    pub(crate) verbs: Vec<String>,
    #[serde(default)]
    pub(crate) users: Vec<String>,
    #[serde(default)]
    pub(crate) user_groups: Vec<String>,
    #[serde(default)]
    pub(crate) namespaces: Vec<String>,
    #[serde(default)]
    pub(crate) resources: Vec<AuditResourceSelector>,
    #[serde(default)]
    pub(crate) non_resource_urls: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AuditResourceSelector {
    #[serde(default)]
    pub(crate) group: String,
    #[serde(default)]
    pub(crate) resources: Vec<String>,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum AuditLevel {
    #[default]
    None,
    Metadata,
    Request,
    RequestResponse,
}

fn default_anonymous_enabled() -> bool {
    false
}

fn default_service_account_token_ttl_seconds() -> u64 {
    3600
}

fn default_service_account_token_max_ttl_seconds() -> u64 {
    86400
}

fn default_service_account_token_leeway_seconds() -> u64 {
    60
}

fn default_audit_log_path() -> String {
    "-".to_string()
}

fn default_audit_max_size_mb() -> u64 {
    100
}

fn default_audit_max_backups() -> usize {
    5
}

fn default_audit_max_age_days() -> u64 {
    30
}

fn default_audit_channel_capacity() -> usize {
    1024
}

fn default_audit_max_request_body_bytes() -> usize {
    256 * 1024
}

fn default_audit_max_response_body_bytes() -> usize {
    256 * 1024
}

impl Default for AuthenticationConfig {
    fn default() -> Self {
        Self {
            anonymous_enabled: default_anonymous_enabled(),
            service_account: ServiceAccountTokenConfig::default(),
        }
    }
}

impl crate::ApiServer {
    pub async fn from_config(
        value: ApiServerConfig,
    ) -> Result<Self, tugboat_resource_store::error::Error> {
        let store = if let Some(tls) = value.etcd.tls {
            ResourceStore::new(value.etcd.endpoints.as_slice(), tls.into()).await?
        } else if value.etcd.allow_insecure_etcd {
            ResourceStore::new_insecure(value.etcd.endpoints.as_slice()).await?
        } else {
            return Err(tugboat_resource_store::error::Error::InvalidField(
                "etcd.tls".to_string(),
                "TLS configuration is missing and allow_insecure_etcd is false. Refusing to connect to etcd in insecure mode.".to_string(),
            ));
        };
        let service_account_tokens =
            crate::auth::service_account_jwt::ServiceAccountTokenIssuer::from_config(
                &value.authentication.service_account,
            )
            .map_err(|reason| {
                tugboat_resource_store::error::Error::InvalidField(
                    "authentication.service_account".to_string(),
                    reason,
                )
            })?;
        let operator = ApiOperator::new(store, service_account_tokens);
        Ok(Self::new(
            value.http.listen,
            operator,
            value.http.tls,
            value.authentication,
            value.authorization,
            value.audit,
            value.http.allow_insecure_http,
        ))
    }
}

impl ApiServerConfig {
    pub fn new(listen: impl Into<String>, etcd_endpoints: Vec<String>) -> Self {
        Self {
            http: HttpConfig {
                listen: listen.into(),
                tls: None,
                allow_insecure_http: false,
            },
            etcd: EtcdConfig {
                endpoints: etcd_endpoints,
                tls: None,
                allow_insecure_etcd: false,
            },
            authentication: AuthenticationConfig::default(),
            authorization: AuthorizationConfig::default(),
            audit: AuditConfig::default(),
        }
    }

    pub fn load_from_file(file: impl AsRef<std::path::Path>) -> Result<Self, ConfigLoadError> {
        load_component_toml_config("tugboat-apiserver", file)
    }
}

impl From<EtcdTlsConfigToml> for EtcdTlsConfig {
    fn from(value: EtcdTlsConfigToml) -> Self {
        Self {
            ca_cert_path: value.ca_cert_path.into(),
            cert_path: value.cert_path.into(),
            key_path: value.key_path.into(),
            domain_name: value.domain_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiServerConfig, AuthorizationMode};

    #[test]
    fn authorization_mode_defaults_to_rbac() {
        assert!(matches!(
            AuthorizationMode::default(),
            AuthorizationMode::Rbac
        ));
    }

    #[test]
    fn new_config_defaults_to_rbac_authorization() {
        let config =
            ApiServerConfig::new("127.0.0.1:8443", vec!["http://127.0.0.1:2379".to_string()]);

        assert!(matches!(config.authorization.mode, AuthorizationMode::Rbac));
    }

    #[test]
    fn anonymous_auth_defaults_to_disabled() {
        let config =
            ApiServerConfig::new("127.0.0.1:8443", vec!["http://127.0.0.1:2379".to_string()]);

        assert!(!config.authentication.anonymous_enabled);
    }

    #[test]
    fn sample_config_deserializes() {
        let config = ApiServerConfig::load_from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sample-configs/apiserver/config.toml"
        ))
        .unwrap();

        assert_eq!(config.http.listen, "0.0.0.0:8443");
        assert_eq!(config.etcd.endpoints, vec!["https://etcd:2379"]);
        assert!(matches!(config.authorization.mode, AuthorizationMode::Rbac));
    }

    #[test]
    fn installer_config_template_deserializes_after_rendering() {
        let template = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../installer/systemd/configs/apiserver.config.toml.tpl"
        ));
        let rendered = template
            .replace("${APISERVER_LISTEN}", "0.0.0.0:8443")
            .replace(
                "${APISERVER_TLS_CONFIG}",
                "[http.tls]\ncert_file = \"/etc/tugboat/pki/apiserver.crt\"\nkey_file = \"/etc/tugboat/pki/apiserver.key\"",
            )
            .replace("${APISERVER_ETCD_ENDPOINTS}", "\"https://etcd:2379\"")
            .replace(
                "${APISERVER_ETCD_TLS_CONFIG}",
                "[etcd.tls]\nca_cert_path = \"/etc/tugboat/pki/etcd/ca.crt\"\ncert_path = \"/etc/tugboat/pki/etcd/client.crt\"\nkey_path = \"/etc/tugboat/pki/etcd/client.key\"",
            )
            .replace("${APISERVER_AUTHORIZATION_MODE}", "RBAC")
            .replace("${APISERVER_ANONYMOUS_ENABLED}", "false")
            .replace("${APISERVER_SERVICE_ACCOUNT_TOKEN_CONFIG}", "")
            .replace("${APISERVER_AUDIT_CONFIG}", "");

        let config: ApiServerConfig = toml::from_str(&rendered).unwrap();

        assert_eq!(config.http.listen, "0.0.0.0:8443");
        assert_eq!(config.etcd.endpoints, vec!["https://etcd:2379"]);
    }
}
