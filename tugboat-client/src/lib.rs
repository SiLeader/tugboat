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

use reqwest::Identity;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs;
use tugboat_resources::StaticResource;
use tugboat_resources::manifests::meta::v1::Time;
use url::Url;

mod api;
mod error;
mod reflector;
mod response;
pub mod runtime;
mod watch;

pub use api::*;
pub use error::*;
pub use response::ListResponse;
pub use watch::*;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccountTokenRequest {
    #[serde(default)]
    pub audiences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_object_ref: Option<BoundObjectReference>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundObjectReference {
    pub kind: String,
    pub api_version: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccountTokenResponse {
    pub token: String,
    pub expiration_timestamp: Time,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientAuth {
    #[default]
    #[serde(alias = "anonymous")]
    None,
    BearerToken {
        token: String,
    },
    ServiceAccount {
        token_path: String,
    },
    ClientCertificate {
        cert_path: String,
        key_path: String,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientTlsConfig {
    pub ca_cert_path: Option<String>,
}

#[derive(Clone)]
pub struct TugboatClient {
    base_url: Url,
    client: reqwest::Client,
}

impl TugboatClient {
    /// Creates an anonymous client. Secret and ServiceAccount operations require
    /// HTTPS even when other resources are accessed over HTTP.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::try_new(base_url, ClientAuth::None, ClientTlsConfig::default())
            .expect("default tugboat client configuration should be valid")
    }

    pub fn try_new(
        base_url: impl Into<String>,
        auth: ClientAuth,
        tls: ClientTlsConfig,
    ) -> Result<Self, Error> {
        let base_url_str = base_url.into();
        let base_url = Url::parse(&base_url_str)?;
        let is_https = base_url.scheme() == "https";
        if !is_https && !allows_cleartext(&auth, &tls) {
            return Err(Error::InsecureUrl(base_url_str));
        }
        Ok(Self {
            client: build_http_client(auth, tls, !is_https)?,
            base_url,
        })
    }
}

fn allows_cleartext(auth: &ClientAuth, tls: &ClientTlsConfig) -> bool {
    matches!(auth, ClientAuth::None) && tls.ca_cert_path.is_none()
}

fn build_http_client(
    auth: ClientAuth,
    tls: ClientTlsConfig,
    allow_cleartext: bool,
) -> Result<reqwest::Client, Error> {
    let mut builder = reqwest::Client::builder().https_only(!allow_cleartext);
    match auth {
        ClientAuth::None => {}
        ClientAuth::BearerToken { token } => {
            builder = builder.default_headers(bearer_headers(token)?);
        }
        ClientAuth::ServiceAccount { token_path } => {
            let token = fs::read_to_string(token_path)?.trim().to_string();
            builder = builder.default_headers(bearer_headers(token)?);
        }
        ClientAuth::ClientCertificate {
            cert_path,
            key_path,
        } => {
            let cert = fs::read(cert_path)?;
            let key = fs::read(key_path)?;
            let mut pem = cert;
            pem.extend_from_slice(&key);
            let identity = Identity::from_pem(&pem)?;
            builder = builder.identity(identity);
        }
    }
    if let Some(ca_cert_path) = tls.ca_cert_path {
        let ca_cert = fs::read(ca_cert_path)?;
        let cert = reqwest::Certificate::from_pem(&ca_cert)?;
        builder = builder.add_root_certificate(cert);
    }
    Ok(builder.build()?)
}

fn bearer_headers(token: String) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&format!("Bearer {token}"))?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

impl TugboatClient {
    // Anonymous HTTP remains available for local installation, but authentication
    // settings do not determine whether a resource contains confidential data.
    fn resource_client<T: StaticResource>(&self) -> Result<&reqwest::Client, Error> {
        if matches!(T::plural(), "secrets" | "serviceaccounts") && self.base_url.scheme() != "https"
        {
            return Err(Error::InsecureUrl(self.base_url.to_string()));
        }
        // HTTPS base URLs always use an https_only client, including redirects.
        Ok(&self.client)
    }

    fn api_path<T: StaticResource>() -> String {
        let group = T::group();
        if group == "core" || group.is_empty() {
            format!("/api/{}", T::version())
        } else {
            format!("/apis/{}/{}", group, T::version())
        }
    }

    fn build_url(&self, path: &str) -> Url {
        let mut url = self.base_url.clone();
        url.set_path(path);
        url
    }

    async fn create_impl<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        path: &str,
        body: T,
    ) -> Result<T, Error> {
        let res = self
            .resource_client::<T>()?
            .post(self.build_url(path))
            .json(&body)
            .send()
            .await?;
        Self::parse_response(res).await
    }

    async fn get_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Option<T>, Error> {
        let res = self
            .resource_client::<T>()?
            .get(self.build_url(path))
            .send()
            .await?;
        Self::parse_response_opt(res).await
    }

    async fn list_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Vec<T>, Error> {
        let res = self
            .resource_client::<T>()?
            .get(self.build_url(path))
            .send()
            .await?;
        Self::parse_response_list(res).await
    }

    async fn list_impl_with_params<T: StaticResource + DeserializeOwned>(
        &self,
        path: &str,
        params: &WatchParams,
    ) -> Result<Vec<T>, Error> {
        let mut url = self.build_url(path);
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(l) = &params.label_selector {
                pairs.append_pair("labelSelector", l);
            }
            if let Some(f) = &params.field_selector {
                pairs.append_pair("fieldSelector", f);
            }
        }
        let res = self.resource_client::<T>()?.get(url).send().await?;
        Self::parse_response_list(res).await
    }

    async fn list_impl_with_params_full<T: StaticResource + DeserializeOwned>(
        &self,
        path: &str,
        params: &WatchParams,
    ) -> Result<response::ListResponse<T>, Error> {
        let mut url = self.build_url(path);
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(l) = &params.label_selector {
                pairs.append_pair("labelSelector", l);
            }
            if let Some(f) = &params.field_selector {
                pairs.append_pair("fieldSelector", f);
            }
        }
        let res = self.resource_client::<T>()?.get(url).send().await?;
        Self::parse_response_list_full(res).await
    }

    async fn patch_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: &str,
        body: impl Serialize,
    ) -> Result<T, Error> {
        let res = self
            .resource_client::<T>()?
            .patch(self.build_url(path))
            .json(&body)
            .send()
            .await?;
        Self::parse_response(res).await
    }

    async fn put_impl<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        path: &str,
        body: T,
    ) -> Result<T, Error> {
        let res = self
            .resource_client::<T>()?
            .put(self.build_url(path))
            .json(&body)
            .send()
            .await?;
        Self::parse_response(res).await
    }

    async fn delete_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Option<T>, Error> {
        let res = self
            .resource_client::<T>()?
            .delete(self.build_url(path))
            .send()
            .await?;
        Self::parse_response_opt(res).await
    }

    pub async fn create_service_account_token(
        &self,
        namespace: &str,
        name: &str,
        request: ServiceAccountTokenRequest,
    ) -> Result<ServiceAccountTokenResponse, Error> {
        let path = format!("/api/v1/namespaces/{namespace}/serviceaccounts/{name}/token");
        let res = self
            .resource_client::<tugboat_resources::manifests::core::v1::ServiceAccount>()?
            .post(self.build_url(&path))
            .json(&request)
            .send()
            .await?;
        Self::parse_response(res).await
    }

    pub(crate) async fn create_cluster_scoped<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        resource: T,
    ) -> Result<T, Error> {
        let path = format!("{}/{}", Self::api_path::<T>(), T::plural());
        self.create_impl(&path, resource).await
    }

    pub(crate) async fn get_cluster_scoped<T: StaticResource + DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!("{}/{}/{name}", Self::api_path::<T>(), T::plural());
        self.get_impl(&path).await
    }

    pub(crate) async fn list_cluster_scoped<T: StaticResource + DeserializeOwned>(
        &self,
    ) -> Result<Vec<T>, Error> {
        let path = format!("{}/{}", Self::api_path::<T>(), T::plural());
        self.list_impl(&path).await
    }

    pub(crate) async fn list_cluster_scoped_with_params<T: StaticResource + DeserializeOwned>(
        &self,
        params: &WatchParams,
    ) -> Result<Vec<T>, Error> {
        let path = format!("{}/{}", Self::api_path::<T>(), T::plural());
        self.list_impl_with_params(&path, params).await
    }

    pub(crate) async fn list_cluster_scoped_with_params_full<
        T: StaticResource + DeserializeOwned,
    >(
        &self,
        params: &WatchParams,
    ) -> Result<response::ListResponse<T>, Error> {
        let path = format!("{}/{}", Self::api_path::<T>(), T::plural());
        self.list_impl_with_params_full(&path, params).await
    }

    pub(crate) async fn patch_cluster_scoped<T: StaticResource + DeserializeOwned, P: Serialize>(
        &self,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{name}", Self::api_path::<T>(), T::plural());
        self.patch_impl(&path, patch).await
    }

    pub(crate) async fn patch_status_cluster_scoped<
        T: StaticResource + DeserializeOwned,
        P: Serialize,
    >(
        &self,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{name}/status", Self::api_path::<T>(), T::plural());
        self.patch_impl(&path, patch).await
    }

    pub(crate) async fn replace_status_cluster_scoped<
        T: StaticResource + Serialize + DeserializeOwned,
    >(
        &self,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{name}/status", Self::api_path::<T>(), T::plural());
        self.put_impl(&path, data).await
    }

    pub(crate) async fn delete_cluster_scoped<T: StaticResource + DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!("{}/{}/{name}", Self::api_path::<T>(), T::plural());
        self.delete_impl(&path).await
    }

    pub(crate) async fn create_namespaced<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        namespace: &str,
        resource: T,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.create_impl(&path, resource).await
    }

    pub(crate) async fn get_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.get_impl(&path).await
    }

    pub(crate) async fn list_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
    ) -> Result<Vec<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.list_impl(&path).await
    }

    pub(crate) async fn list_namespaced_with_params<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        params: &WatchParams,
    ) -> Result<Vec<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.list_impl_with_params(&path, params).await
    }

    pub(crate) async fn list_namespaced_with_params_full<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        params: &WatchParams,
    ) -> Result<response::ListResponse<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.list_impl_with_params_full(&path, params).await
    }

    pub(crate) async fn patch_namespaced<T: StaticResource + DeserializeOwned, P: Serialize>(
        &self,
        namespace: &str,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.patch_impl(&path, patch).await
    }

    pub(crate) async fn patch_status_namespaced<
        T: StaticResource + DeserializeOwned,
        P: Serialize,
    >(
        &self,
        namespace: &str,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}/status",
            Self::api_path::<T>(),
            T::plural()
        );
        self.patch_impl(&path, patch).await
    }

    pub(crate) async fn replace_status_namespaced<
        T: StaticResource + Serialize + DeserializeOwned,
    >(
        &self,
        namespace: &str,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}/status",
            Self::api_path::<T>(),
            T::plural()
        );
        self.put_impl(&path, data).await
    }

    pub(crate) async fn replace_cluster_scoped<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{name}", Self::api_path::<T>(), T::plural());
        self.put_impl(&path, data).await
    }

    pub(crate) async fn replace_namespaced<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        namespace: &str,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.put_impl(&path, data).await
    }

    pub(crate) async fn delete_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}",
            Self::api_path::<T>(),
            T::plural()
        );
        self.delete_impl(&path).await
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientAuth, ClientTlsConfig, TugboatClient};

    fn assert_insecure<T>(result: Result<T, super::Error>) {
        assert!(matches!(result, Err(super::Error::InsecureUrl(_))));
    }

    async fn assert_sensitive_api_rejects_http<T>()
    where
        T: tugboat_resources::NamespacedResource
            + serde::Serialize
            + serde::de::DeserializeOwned
            + Default,
    {
        let client = TugboatClient::new("http://127.0.0.1:1");
        // Cover both namespaced and all-namespace access, including reflector lists.
        for api in [
            super::Api::<T>::namespaced(client.clone(), "default"),
            super::Api::<T>::all(client.clone()),
        ] {
            let params = super::WatchParams::default().labels("app=test");
            assert_insecure(api.create(T::default()).await);
            assert_insecure(api.get("test").await);
            assert_insecure(api.list().await);
            assert_insecure(api.list_with_params(&params).await);
            assert_insecure(api.list_with_params_full(&params).await);
            assert_insecure(api.patch("test", serde_json::json!({})).await);
            assert_insecure(api.patch_status("test", serde_json::json!({})).await);
            assert_insecure(api.replace("test", T::default()).await);
            assert_insecure(api.replace_status("test", T::default()).await);
            assert_insecure(api.delete("test").await);
            assert_insecure(api.watch_raw(params).await);
        }
    }

    #[tokio::test]
    async fn sensitive_operations_reject_anonymous_http() {
        use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};

        assert_sensitive_api_rejects_http::<Secret>().await;
        assert_sensitive_api_rejects_http::<ServiceAccount>().await;
        assert_insecure(
            TugboatClient::new("http://127.0.0.1:1")
                .create_service_account_token("default", "test", Default::default())
                .await,
        );
    }

    #[tokio::test]
    async fn sensitive_https_client_rejects_cleartext_requests() {
        use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};

        let client = TugboatClient::new("https://127.0.0.1:1");
        assert!(client.resource_client::<ServiceAccount>().is_ok());
        let error = client
            .resource_client::<Secret>()
            .unwrap()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        assert!(
            error.is_builder(),
            "HTTP must be rejected before connecting"
        );
    }

    #[test]
    fn ordinary_resources_still_allow_anonymous_http() {
        use tugboat_resources::manifests::core::v1::Ship;

        let client = TugboatClient::new("http://127.0.0.1:1");
        assert!(client.resource_client::<Ship>().is_ok());
    }

    #[test]
    fn anonymous_http_url_is_allowed() {
        TugboatClient::try_new(
            "http://127.0.0.1:8080",
            ClientAuth::None,
            ClientTlsConfig::default(),
        )
        .expect("anonymous cleartext client should be allowed for local installers");
    }

    #[test]
    fn authenticated_http_url_is_rejected() {
        let err = match TugboatClient::try_new(
            "http://127.0.0.1:8080",
            ClientAuth::BearerToken {
                token: "secret".to_string(),
            },
            ClientTlsConfig::default(),
        ) {
            Ok(_) => panic!("authenticated cleartext client should be rejected"),
            Err(err) => err,
        };

        assert_eq!(
            err.to_string(),
            "tugboat client requires an HTTPS URL: http://127.0.0.1:8080"
        );
    }
}
