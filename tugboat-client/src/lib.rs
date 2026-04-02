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

use serde::Serialize;
use serde::de::DeserializeOwned;
use tugboat_resources::StaticResource;
use url::Url;

mod api;
mod error;
mod reflector;
mod response;
pub mod runtime;
mod watch;

pub use api::*;
pub use error::*;
pub use watch::*;

#[derive(Clone)]
pub struct TugboatClient {
    base_url: String,
    client: reqwest::Client,
}

impl TugboatClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

impl TugboatClient {
    fn api_prefix<T: StaticResource>(&self) -> String {
        let group = T::group();
        if group == "core" || group.is_empty() {
            format!("{}/api/{}", self.base_url, T::version())
        } else {
            format!("{}/apis/{}/{}", self.base_url, group, T::version())
        }
    }

    async fn create_impl<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        path: String,
        body: T,
    ) -> Result<T, Error> {
        let res = self.client.post(path).json(&body).send().await?;
        Self::parse_response(res).await
    }

    async fn get_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: String,
    ) -> Result<Option<T>, Error> {
        let res = self.client.get(path).send().await?;
        Self::parse_response_opt(res).await
    }

    async fn list_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: String,
    ) -> Result<Vec<T>, Error> {
        let res = self.client.get(path).send().await?;
        Self::parse_response_list(res).await
    }

    async fn list_impl_with_params<T: StaticResource + DeserializeOwned>(
        &self,
        path: String,
        params: &WatchParams,
    ) -> Result<Vec<T>, Error> {
        let url = Url::parse_with_params(
            &path,
            [
                params
                    .label_selector
                    .as_ref()
                    .map(|l| ("labelSelector", l.as_str())),
                params
                    .field_selector
                    .as_ref()
                    .map(|f| ("fieldSelector", f.as_str())),
            ]
            .into_iter()
            .flatten(),
        )?;
        let res = self.client.get(url.as_str()).send().await?;
        Self::parse_response_list(res).await
    }

    async fn patch_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: String,
        body: impl Serialize,
    ) -> Result<T, Error> {
        let res = self.client.patch(path).json(&body).send().await?;
        Self::parse_response(res).await
    }

    async fn put_impl<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        path: String,
        body: T,
    ) -> Result<T, Error> {
        let res = self.client.put(path).json(&body).send().await?;
        Self::parse_response(res).await
    }

    async fn delete_impl<T: StaticResource + DeserializeOwned>(
        &self,
        path: String,
    ) -> Result<Option<T>, Error> {
        let res = self.client.delete(path).send().await?;
        Self::parse_response_opt(res).await
    }

    pub(crate) async fn create_cluster_scoped<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        resource: T,
    ) -> Result<T, Error> {
        let path = format!("{}/{}", self.api_prefix::<T>(), T::plural());
        self.create_impl(path, resource).await
    }

    pub(crate) async fn get_cluster_scoped<T: StaticResource + DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!("{}/{}/{name}", self.api_prefix::<T>(), T::plural());
        self.get_impl(path).await
    }

    pub(crate) async fn list_cluster_scoped<T: StaticResource + DeserializeOwned>(
        &self,
    ) -> Result<Vec<T>, Error> {
        let path = format!("{}/{}", self.api_prefix::<T>(), T::plural());
        self.list_impl(path).await
    }

    pub(crate) async fn list_cluster_scoped_with_params<T: StaticResource + DeserializeOwned>(
        &self,
        params: &WatchParams,
    ) -> Result<Vec<T>, Error> {
        let path = format!("{}/{}", self.api_prefix::<T>(), T::plural());
        self.list_impl_with_params(path, params).await
    }

    pub(crate) async fn patch_cluster_scoped<T: StaticResource + DeserializeOwned, P: Serialize>(
        &self,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{name}", self.api_prefix::<T>(), T::plural());
        self.patch_impl(path, patch).await
    }

    pub(crate) async fn patch_status_cluster_scoped<
        T: StaticResource + DeserializeOwned,
        P: Serialize,
    >(
        &self,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{name}/status", self.api_prefix::<T>(), T::plural());
        self.patch_impl(path, patch).await
    }

    pub(crate) async fn replace_status_cluster_scoped<
        T: StaticResource + Serialize + DeserializeOwned,
    >(
        &self,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        self.put_impl(
            format!("{}/{}/{name}/status", self.api_prefix::<T>(), T::plural()),
            data,
        )
        .await
    }

    pub(crate) async fn delete_cluster_scoped<T: StaticResource + DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Option<T>, Error> {
        self.delete_impl(format!(
            "{}/{}/{}",
            self.api_prefix::<T>(),
            T::plural(),
            name
        ))
        .await
    }

    pub(crate) async fn create_namespaced<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        namespace: &str,
        resource: T,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            self.api_prefix::<T>(),
            T::plural()
        );
        self.create_impl(path, resource).await
    }

    pub(crate) async fn get_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}",
            self.api_prefix::<T>(),
            T::plural()
        );
        self.get_impl(path).await
    }

    pub(crate) async fn list_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
    ) -> Result<Vec<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            self.api_prefix::<T>(),
            T::plural()
        );
        self.list_impl(path).await
    }

    pub(crate) async fn list_namespaced_with_params<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        params: &WatchParams,
    ) -> Result<Vec<T>, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}",
            self.api_prefix::<T>(),
            T::plural()
        );
        self.list_impl_with_params(path, params).await
    }

    pub(crate) async fn patch_namespaced<T: StaticResource + DeserializeOwned, P: Serialize>(
        &self,
        namespace: &str,
        name: &str,
        patch: P,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/namespaces/{namespace}/{}/{name}",
            self.api_prefix::<T>(),
            T::plural()
        );
        self.patch_impl(path, patch).await
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
            self.api_prefix::<T>(),
            T::plural()
        );
        self.patch_impl(path, patch).await
    }

    pub(crate) async fn replace_status_namespaced<
        T: StaticResource + Serialize + DeserializeOwned,
    >(
        &self,
        namespace: &str,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        self.put_impl(
            format!(
                "{}/namespaces/{namespace}/{}/{name}/status",
                self.api_prefix::<T>(),
                T::plural()
            ),
            data,
        )
        .await
    }

    pub(crate) async fn replace_cluster_scoped<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        self.put_impl(
            format!("{}/{}/{name}", self.api_prefix::<T>(), T::plural()),
            data,
        )
        .await
    }

    pub(crate) async fn replace_namespaced<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        namespace: &str,
        name: &str,
        data: T,
    ) -> Result<T, Error> {
        self.put_impl(
            format!(
                "{}/namespaces/{namespace}/{}/{name}",
                self.api_prefix::<T>(),
                T::plural()
            ),
            data,
        )
        .await
    }

    pub(crate) async fn delete_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<T>, Error> {
        self.delete_impl(format!(
            "{}/namespaces/{namespace}/{}/{name}",
            self.api_prefix::<T>(),
            T::plural()
        ))
        .await
    }
}
