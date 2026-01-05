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

mod api;
mod error;
mod response;
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

    pub(crate) async fn create_cluster_wide<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        resource: T,
    ) -> Result<T, Error> {
        let path = format!("{}/{}/{}", self.base_url, T::version(), T::plural());
        self.create_impl(path, resource).await
    }

    pub(crate) async fn get_cluster_wide<T: StaticResource + DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let path = format!("{}/{}/{}/{name}", self.base_url, T::version(), T::plural());
        self.get_impl(path).await
    }

    pub(crate) async fn list_cluster_wide<T: StaticResource + DeserializeOwned>(
        &self,
    ) -> Result<Vec<T>, Error> {
        let path = format!("{}/{}/{}", self.base_url, T::version(), T::plural());
        self.list_impl(path).await
    }

    pub(crate) async fn create_namespaced<T: StaticResource + Serialize + DeserializeOwned>(
        &self,
        namespace: &str,
        resource: T,
    ) -> Result<T, Error> {
        let path = format!(
            "{}/{}/namespaces/{namespace}/{}",
            self.base_url,
            T::version(),
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
            "{}/{}/namespaces/{namespace}/{}/{name}",
            self.base_url,
            T::version(),
            T::plural()
        );
        self.get_impl(path).await
    }

    pub(crate) async fn list_namespaced<T: StaticResource + DeserializeOwned>(
        &self,
        namespace: &str,
    ) -> Result<Vec<T>, Error> {
        let path = format!(
            "{}/{}/namespaces/{namespace}/{}",
            self.base_url,
            T::version(),
            T::plural()
        );
        self.list_impl(path).await
    }
}
