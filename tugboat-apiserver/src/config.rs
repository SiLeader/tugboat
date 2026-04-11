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
use tugboat_resource_store::ResourceStore;

#[derive(serde::Deserialize)]
pub struct ApiServerConfig {
    http: HttpConfig,
    etcd: EtcdConfig,
    #[serde(default)]
    authentication: AuthenticationConfig,
    #[serde(default)]
    authorization: AuthorizationConfig,
}

#[derive(serde::Deserialize)]
pub(crate) struct EtcdConfig {
    endpoints: Vec<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct HttpConfig {
    listen: String,
    tls: Option<TlsConfig>,
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

fn default_anonymous_enabled() -> bool {
    false
}

impl Default for AuthenticationConfig {
    fn default() -> Self {
        Self {
            anonymous_enabled: default_anonymous_enabled(),
        }
    }
}

impl crate::ApiServer {
    pub async fn from_config(
        value: ApiServerConfig,
    ) -> Result<Self, tugboat_resource_store::error::Error> {
        let store = ResourceStore::new(value.etcd.endpoints.as_slice()).await?;
        let operator = ApiOperator::new(store);
        Ok(Self::new(
            value.http.listen,
            operator,
            value.http.tls,
            value.authentication,
            value.authorization,
        ))
    }
}

impl ApiServerConfig {
    pub fn new(listen: impl Into<String>, etcd_endpoints: Vec<String>) -> Self {
        Self {
            http: HttpConfig {
                listen: listen.into(),
                tls: None,
            },
            etcd: EtcdConfig {
                endpoints: etcd_endpoints,
            },
            authentication: AuthenticationConfig::default(),
            authorization: AuthorizationConfig::default(),
        }
    }

    pub fn load_from_file(
        file: impl AsRef<std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = file.as_ref();
        let file = std::fs::read_to_string(path).map_err(|e| {
            std::io::Error::other(format!("Failed to read config file {:?}: {e}", path))
        })?;
        toml::from_str(&file).map_err(|e| {
            std::io::Error::other(format!("Failed to parse config file {:?}: {e}", path)).into()
        })
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
}
