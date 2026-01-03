use crate::error::Error;
use crate::serializer::StaticSerializable;
use crate::watch::WatchReceiver;
use etcd_client::{Client, Compare, CompareOp, GetOptions, Txn, TxnOp, TxnOpResponse};
use tugboat_resources::manifests::meta::v1::ObjectMeta;
use tugboat_resources::{ObjectMetaResource, StaticResource};

pub mod error;
pub mod serializer;
pub mod watch;
mod watch_reflector;

pub struct ResourceStore {
    etcd: Client,
    watch_mux: watch::WatchMuxAggregator,
}

const BASE_PATH: &str = "/tugboat/registry";

pub struct ContentData<T> {
    pub data: T,
    pub revision: i64,
}

impl<T: ObjectMetaResource> ContentData<T> {
    pub fn apply_revision(mut self) -> T {
        let rev = self.revision;
        self.data.modify_object_meta(|meta| match meta {
            None => {
                let _ = meta.insert(ObjectMeta {
                    generation: Some(rev),
                    resource_version: Some(rev.to_string()),
                    ..Default::default()
                });
            }
            Some(meta) => meta.generation = Some(rev),
        });
        self.data
    }
}

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
    ) -> Result<ContentData<T>, Error> {
        let Some(meta) = value.object_meta() else {
            return Err(Error::FieldMissing("metadata".to_string()));
        };
        let Some(name) = &meta.name else {
            return Err(Error::FieldMissing("metadata.name".to_string()));
        };
        let key = Self::create_key::<T>(meta.namespace.clone(), name);
        let bytes = value.serialize()?;

        let mut client = self.etcd.clone();
        let res = client.put(key, bytes, None).await?;

        Ok(ContentData {
            data: value,
            revision: res.header().map(|h| h.revision()).unwrap_or(0),
        })
    }

    pub async fn put_if_not_exists<T: StaticSerializable + ObjectMetaResource>(
        &self,
        value: T,
    ) -> Result<Option<ContentData<T>>, Error> {
        let Some(meta) = value.object_meta() else {
            return Err(Error::FieldMissing("metadata".to_string()));
        };
        let Some(name) = &meta.name else {
            return Err(Error::FieldMissing("metadata.name".to_string()));
        };
        let key = Self::create_key::<T>(meta.namespace.clone(), name);
        let bytes = value.serialize()?;

        let txn = Txn::new()
            .when(vec![Compare::create_revision(
                key.as_str(),
                CompareOp::Equal,
                0,
            )])
            .and_then(vec![TxnOp::put(key.as_str(), bytes, None)]);
        let mut client = self.etcd.clone();
        let res = client.txn(txn).await?;
        if let Some(TxnOpResponse::Put(txn_res)) = res.op_responses().first()
            && res.succeeded()
        {
            Ok(Some(ContentData {
                data: value,
                revision: txn_res.header().map(|h| h.revision()).unwrap_or(0),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        name: &str,
    ) -> Result<Option<ContentData<T>>, Error> {
        let key = Self::create_key::<T>(namespace, name);

        let mut client = self.etcd.clone();
        let res = client.get(key, None).await?;

        let Some(kv) = res.kvs().first() else {
            return Ok(None);
        };
        let value = T::deserialize(kv.value())?;
        Ok(Some(ContentData {
            data: value,
            revision: res.header().map(|h| h.revision()).unwrap_or(0),
        }))
    }

    pub async fn list<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        limit: Option<usize>,
    ) -> Result<Vec<ContentData<T>>, Error> {
        let key = Self::create_key::<T>(namespace, "");
        let mut client = self.etcd.clone();
        let mut options = GetOptions::default().with_prefix();
        if let Some(limit) = limit {
            options = options.with_limit(limit as i64);
        }

        let res = client.get(key, Some(options)).await?;
        let mut data = Vec::new();
        for kv in res.kvs() {
            let value = T::deserialize(kv.value())?;
            data.push(ContentData {
                data: value,
                revision: res.header().map(|h| h.revision()).unwrap_or(0),
            });
        }
        Ok(data)
    }

    pub async fn watch<T: StaticSerializable>(
        &self,
        resource_version: Option<String>,
    ) -> Result<WatchReceiver, Error> {
        let key = Self::create_watch_key::<T>();
        self.watch_mux.get(&key, resource_version).await
    }
}
