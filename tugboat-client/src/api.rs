use crate::TugboatClient;
use crate::error::Error;
use crate::watch::WatchEvent;
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
            self.client.create_cluster_wide(resource).await
        }
    }

    pub async fn get(&self, name: &str) -> Result<Option<T>, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.get_namespaced(namespace, name).await
        } else {
            self.client.get_cluster_wide(name).await
        }
    }

    pub async fn list(&self) -> Result<Vec<T>, Error> {
        if let Some(namespace) = &self.namespace {
            self.client.list_namespaced(namespace).await
        } else {
            self.client.list_cluster_wide().await
        }
    }

    pub async fn watch(&self) -> Result<impl Stream<Item = Result<WatchEvent<T>, Error>>, Error> {
        let path = if let Some(namespace) = &self.namespace {
            format!(
                "{}/namespaces/{namespace}/{}?watch=true",
                T::version(),
                T::plural()
            )
        } else {
            format!("{}/{}?watch=true", T::version(), T::plural())
        };
        self.client.watch_impl(path).await
    }
}
