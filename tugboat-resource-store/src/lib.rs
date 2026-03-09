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

use crate::error::Error;
use crate::serializer::StaticSerializable;
use crate::watch::WatchReceiver;
use etcd_client::{
    Client, Compare, CompareOp, DeleteOptions, GetOptions, Txn, TxnOp, TxnOpResponse,
};
use tracing::{debug, info};
use tugboat_resources::manifests::meta::v1::ObjectMeta;
use tugboat_resources::{ObjectMetaResource, StaticResource};

pub mod error;
pub mod serializer;
pub mod watch;

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
            Some(meta) => {
                meta.generation = Some(rev);
                meta.resource_version = Some(rev.to_string());
            }
        });
        self.data
    }
}

impl ResourceStore {
    pub async fn new(endpoints: &[String]) -> Self {
        info!("Creating etcd client: endpoints: {endpoints:?}");
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

    fn create_watch_key<T: StaticResource>(namespace: Option<String>) -> String {
        if let Some(ns) = namespace {
            format!("{BASE_PATH}/{}/{}/{ns}/", T::group(), T::plural())
        } else {
            format!("{BASE_PATH}/{}/{}/", T::group(), T::plural())
        }
    }

    pub async fn put<T: StaticSerializable + ObjectMetaResource>(
        &self,
        value: T,
    ) -> Result<ContentData<T>, Error> {
        info!("Put resource");
        let Some(meta) = value.object_meta() else {
            info!("Resource metadata is missing");
            return Err(Error::FieldMissing("metadata".to_string()));
        };
        let Some(name) = &meta.name else {
            info!("Resource metadata.name is missing");
            return Err(Error::FieldMissing("metadata.name".to_string()));
        };
        let resource_version = meta
            .resource_version
            .as_ref()
            .and_then(|v| v.parse::<i64>().ok());
        let key = Self::create_key::<T>(meta.namespace.clone(), name);
        let bytes = value.serialize()?;
        debug!("Put resource key = {key}, value = {} bytes", bytes.len());

        let mut client = self.etcd.clone();

        let res = match resource_version {
            Some(rv) => {
                debug!("Attempting conditional update for {key} with rv={rv}");
                let txn = Txn::new()
                    .when(vec![Compare::mod_revision(
                        key.as_str(),
                        CompareOp::Equal,
                        rv,
                    )])
                    .and_then(vec![TxnOp::put(key.as_str(), bytes, None)])
                    .or_else(vec![TxnOp::get(key.as_str(), None)]);

                let res = client.txn(txn).await?;

                if !res.succeeded() {
                    let header_rev = res.header().map(|h| h.revision()).unwrap_or(-1);
                    // Extract current mod_revision from the get response in or_else
                    let mut current_mod_rev = -1;
                    if let Some(op_resp) = res.op_responses().first()
                        && let TxnOpResponse::Get(range_resp) = op_resp
                        && let Some(kv) = range_resp.kvs().first()
                    {
                        current_mod_rev = kv.mod_revision();
                    }

                    info!(
                        "Optimistic lock failed for key {key}: expected rv {rv}, actual mod_revision {current_mod_rev}, global rev {header_rev}"
                    );
                    return Err(Error::OptimisticLockFailed(rv));
                }

                if let TxnOpResponse::Put(p) = res
                    .op_responses()
                    .first()
                    .ok_or(Error::OptimisticLockFailed(rv))?
                    .clone()
                {
                    // For PUT, we don't get the new mod_revision directly in the response unless we ask for it.
                    // But the header revision is the global revision, which is NOT the mod_revision (unless it's the only change).
                    // Actually, mod_revision = global revision at the time of modification.
                    // So using header.revision() IS correct for the NEW revision of this key.

                    // Wait, if header.revision() is 100, and this key was modified, its mod_revision will be 100.
                    // So for PUT response, header.revision() matches the new mod_revision of the key.
                    // BUT for GET, we were reading header.revision() which was global revision (e.g. 105),
                    // while the key might have been last modified at 100.
                    // So we were comparing 100 (from key) vs 105 (from header).

                    // So, in PUT response, using header.revision() is likely correct as it represents the revision of the transaction.
                    p
                } else {
                    return Err(Error::OptimisticLockFailed(rv));
                }
            }
            None => client.put(key, bytes, None).await?,
        };

        Ok(ContentData {
            data: value,
            // For simple PUT, header.revision() is the revision of this modification.
            revision: res.header().map(|h| h.revision()).unwrap_or(0),
        })
    }

    pub async fn put_if_not_exists<T: StaticSerializable + ObjectMetaResource>(
        &self,
        value: T,
    ) -> Result<Option<ContentData<T>>, Error> {
        info!("Put resource if not exists");
        let Some(meta) = value.object_meta() else {
            info!("Resource metadata is missing");
            return Err(Error::FieldMissing("metadata".to_string()));
        };
        let Some(name) = &meta.name else {
            info!("Resource metadata.name is missing");
            return Err(Error::FieldMissing("metadata.name".to_string()));
        };
        let key = Self::create_key::<T>(meta.namespace.clone(), name);
        let bytes = value.serialize()?;
        debug!(
            "Put resource (if not exists) key = {key}, value = {} bytes",
            bytes.len()
        );

        let txn = Txn::new()
            .when(vec![Compare::create_revision(
                key.as_str(),
                CompareOp::Equal,
                0,
            )])
            .and_then(vec![TxnOp::put(key.as_str(), bytes, None)]);
        let mut client = self.etcd.clone();
        let res = client.txn(txn).await?;
        debug!("Txn response: {res:?}");
        if let Some(TxnOpResponse::Put(_txn_res)) = res.op_responses().first()
            && res.succeeded()
        {
            Ok(Some(ContentData {
                data: value,
                // For a new key created in this txn, revision = header.revision()
                revision: res.header().map(|h| h.revision()).unwrap_or(0),
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
        info!("Get resource: key = {key}");

        let mut client = self.etcd.clone();
        let res = client.get(key, None).await?;

        let Some(kv) = res.kvs().first() else {
            return Ok(None);
        };
        let value = T::deserialize(kv.value())?;
        Ok(Some(ContentData {
            data: value,
            revision: kv.mod_revision(),
        }))
    }

    pub async fn list<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        limit: Option<usize>,
    ) -> Result<Vec<ContentData<T>>, Error> {
        let key = Self::create_key::<T>(namespace, "");
        info!("List resources: key = {key}");
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
                revision: kv.mod_revision(),
            });
        }
        Ok(data)
    }

    pub async fn watch<T: StaticSerializable>(
        &self,
        resource_version: Option<String>,
        namespace: Option<String>,
    ) -> Result<WatchReceiver, Error> {
        let key = Self::create_watch_key::<T>(namespace);
        info!("Watch resources: key = {key}");
        self.watch_mux.get(&key, resource_version).await
    }

    pub async fn delete<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        name: &str,
    ) -> Result<Option<ContentData<T>>, Error> {
        let key = Self::create_key::<T>(namespace, name);
        info!("Delete resource: key = {key}");

        let mut client = self.etcd.clone();
        let response = client
            .delete(key, Some(DeleteOptions::new().with_prev_key()))
            .await?;

        let Some(kv) = response.prev_kvs().first() else {
            return Ok(None);
        };

        let value = T::deserialize(kv.value())?;
        Ok(Some(ContentData {
            data: value,
            revision: kv.mod_revision(),
        }))
    }
}
