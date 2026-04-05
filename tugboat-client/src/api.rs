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

use crate::TugboatClient;
use crate::error::Error;
use crate::watch::{WatchEvent, WatchParams};
use futures::Stream;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tugboat_resources::{NamespacedResource, StaticResource};

#[derive(Clone)]
pub struct Api<T> {
    client: TugboatClient,
    namespace: Option<String>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Api<T> {
    fn new(client: TugboatClient, namespace: Option<String>) -> Self {
        Self {
            client,
            namespace,
            _phantom: Default::default(),
        }
    }
}

impl<T> Api<T>
where
    T: StaticResource,
{
    pub fn all(client: TugboatClient) -> Self {
        Self::new(client, None)
    }
}

impl<T> Api<T>
where
    T: NamespacedResource,
{
    pub fn namespaced(client: TugboatClient, namespace: &str) -> Self {
        Self::new(client, Some(namespace.to_string()))
    }
}

impl<T> Api<T>
where
    T: StaticResource + Serialize + DeserializeOwned,
{
    pub async fn create(&self, resource: T) -> Result<T, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.create_namespaced(namespace, resource).await
        } else {
            self.client.create_cluster_scoped(resource).await
        }
    }

    pub async fn get(&self, name: &str) -> Result<Option<T>, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.get_namespaced(namespace, name).await
        } else {
            self.client.get_cluster_scoped(name).await
        }
    }

    pub(crate) async fn get_with_optional_namespace(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Option<T>, Error> {
        if let Some(namespace) = namespace {
            self.client.get_namespaced(namespace, name).await
        } else {
            self.get(name).await
        }
    }

    pub async fn list(&self) -> Result<Vec<T>, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.list_namespaced(namespace).await
        } else {
            self.client.list_cluster_scoped().await
        }
    }

    pub async fn list_with_params(&self, params: &WatchParams) -> Result<Vec<T>, Error> {
        if let Some(namespace) = &self.namespace {
            self.client
                .list_namespaced_with_params(namespace, params)
                .await
        } else {
            self.client.list_cluster_scoped_with_params(params).await
        }
    }

    pub async fn patch<P: Serialize>(&self, name: &str, patch: P) -> Result<T, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.patch_namespaced(namespace, name, patch).await
        } else {
            self.client.patch_cluster_scoped(name, patch).await
        }
    }

    pub async fn patch_status<P: Serialize>(&self, name: &str, patch: P) -> Result<T, Error> {
        if let Some(namespace) = &self.namespace {
            self.client
                .patch_status_namespaced(namespace, name, patch)
                .await
        } else {
            self.client.patch_status_cluster_scoped(name, patch).await
        }
    }

    pub async fn replace_status(&self, name: &str, data: T) -> Result<T, Error> {
        if let Some(namespace) = &self.namespace {
            self.client
                .replace_status_namespaced(namespace, name, data)
                .await
        } else {
            self.client.replace_status_cluster_scoped(name, data).await
        }
    }

    pub async fn replace(&self, name: &str, data: T) -> Result<T, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.replace_namespaced(namespace, name, data).await
        } else {
            self.client.replace_cluster_scoped(name, data).await
        }
    }

    pub async fn delete(&self, name: &str) -> Result<Option<T>, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.delete_namespaced(namespace, name).await
        } else {
            self.client.delete_cluster_scoped(name).await
        }
    }

    pub async fn watch_raw(
        &self,
        params: &WatchParams,
    ) -> Result<impl Stream<Item = Result<WatchEvent<T>, Error>>, Error> {
        let api_prefix = if T::group() == "core" || T::group().is_empty() {
            format!("api/{}", T::version())
        } else {
            format!("apis/{}/{}", T::group(), T::version())
        };
        let path = if let Some(namespace) = &self.namespace {
            format!(
                "{api_prefix}/namespaces/{namespace}/{}?watch=true",
                T::plural()
            )
        } else {
            format!("{api_prefix}/{}?watch=true", T::plural())
        };
        self.client.watch_impl(path, params).await
    }
}
