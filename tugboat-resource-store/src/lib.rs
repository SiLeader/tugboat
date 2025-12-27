use crate::error::Error;
use crate::serializer::StaticSerializable;
use crate::watch::WatchReceiver;
use etcd_client::Client;
use tugboat_resources::{ObjectMetaResource, StaticResource};

mod error;
mod serializer;
mod watch;

pub struct ResourceStore {
    etcd: Client,
    watch_mux: watch::WatchMuxAggregator,
}

const BASE_PATH: &str = "/tugboat/registry";

impl ResourceStore {
    pub async fn new(endpoints: &[String]) -> Self {
        let client = Client::connect(endpoints, None)
            .await
            .expect("Connect to etcd failed");
        Self {
            etcd: client.clone(),
            watch_mux: watch::WatchMuxAggregator::new(client),
        }
    }

    fn create_key<T: StaticResource>(namespace: Option<String>, name: &str) -> String {
        if T::is_cluster_scoped() {
            format!("{BASE_PATH}/{}/{}/{}", T::group(), T::plural(), name)
        } else {
            let ns = namespace.unwrap_or("default".to_string());
            format!("{BASE_PATH}/{}/{}/{}/{}", T::group(), T::plural(), ns, name)
        }
    }

    fn create_watch_key<T: StaticResource>() -> String {
        format!("{BASE_PATH}/{}/", T::plural())
    }

    pub async fn put<T: StaticSerializable + ObjectMetaResource>(
        &self,
        value: T,
    ) -> Result<(), Error> {
        let Some(meta) = value.object_meta() else {
            return Err(Error::ObjectMetaMissing);
        };
        let key = Self::create_key::<T>(meta.namespace.clone(), &meta.name);
        let bytes = value.serialize()?;

        let mut client = self.etcd.clone();
        client.put(key, bytes, None).await?;
        Ok(())
    }

    pub async fn get<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        name: &str,
    ) -> Result<Option<T>, Error> {
        let key = Self::create_key::<T>(namespace, name);

        let mut client = self.etcd.clone();
        let res = client.get(key, None).await?;

        let Some(kv) = res.kvs().first() else {
            return Ok(None);
        };
        let value = T::deserialize(kv.value())?;
        Ok(Some(value))
    }

    pub async fn watch<T: StaticSerializable>(&self) -> Result<WatchReceiver, Error> {
        let key = Self::create_watch_key::<T>();
        self.watch_mux.get(&key).await
    }
}
