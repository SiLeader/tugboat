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
use crate::serializer::custom_resource::CustomResourceSerializable;
use crate::watch::WatchReceiver;
use etcd_client::{
    Certificate, Client, Compare, CompareOp, ConnectOptions, DeleteOptions, GetOptions, Identity,
    TlsOptions, Txn, TxnOp, TxnOpResponse,
};
use std::path::PathBuf;
use tracing::{debug, info};
use tugboat_resources::manifests::meta::v1::{CustomResourceObject, ObjectMeta};
use tugboat_resources::{ObjectMetaResource, StaticResource};

pub mod error;
pub mod serializer;
pub mod watch;

pub struct ResourceStore {
    etcd: Client,
    watch_mux: watch::WatchMuxAggregator,
}

const BASE_PATH: &str = "/tugboat/registry";

#[derive(Clone, Debug)]
pub struct EtcdTlsConfig {
    pub ca_cert_path: PathBuf,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub domain_name: Option<String>,
}

pub struct ContentData<T> {
    pub data: T,
    pub revision: i64,
}

pub struct PutRequest {
    key: String,
    bytes: Vec<u8>,
    expected_revision: Option<i64>,
}

impl<T: ObjectMetaResource> ContentData<T> {
    pub fn apply_revision(mut self) -> T {
        let rev = self.revision.to_string();
        self.data.modify_object_meta(|meta| match meta {
            None => {
                let _ = meta.insert(ObjectMeta {
                    resource_version: Some(rev.clone()),
                    ..Default::default()
                });
            }
            Some(meta) => {
                meta.resource_version = Some(rev.clone());
            }
        });
        self.data
    }
}

impl ResourceStore {
    fn prepare_put_inner<T: StaticSerializable + ObjectMetaResource>(
        value: &T,
    ) -> Result<PutRequest, Error> {
        let Some(meta) = value.object_meta() else {
            info!("Resource metadata is missing");
            return Err(Error::FieldMissing("metadata".to_string()));
        };
        let Some(name) = &meta.name else {
            info!("Resource metadata.name is missing");
            return Err(Error::FieldMissing("metadata.name".to_string()));
        };
        let expected_revision = match meta.resource_version.as_ref() {
            Some(value) => {
                let parsed = value.parse::<i64>().map_err(|_| {
                    Error::InvalidField(
                        "metadata.resourceVersion".to_string(),
                        "must be a valid integer".to_string(),
                    )
                })?;
                Some(parsed)
            }
            None => None,
        };
        let key = Self::create_key::<T>(meta.namespace.clone(), name)?;
        let bytes = value.serialize()?;
        debug!(
            "Prepared put request key = {key}, value = {} bytes",
            bytes.len()
        );

        Ok(PutRequest {
            key,
            bytes,
            expected_revision,
        })
    }

    pub async fn new_insecure(endpoints: &[String]) -> Result<Self, Error> {
        info!("Creating insecure etcd client: endpoints: {endpoints:?}");
        let client = Client::connect(endpoints, None).await?;
        Ok(Self {
            etcd: client.clone(),
            watch_mux: watch::WatchMuxAggregator::new(client),
        })
    }

    pub async fn new(endpoints: &[String], tls_config: EtcdTlsConfig) -> Result<Self, Error> {
        info!("Creating secure etcd client: endpoints: {endpoints:?}");
        let options = build_connect_options(tls_config)?;
        let client = Client::connect(endpoints, Some(options)).await?;
        Ok(Self {
            etcd: client.clone(),
            watch_mux: watch::WatchMuxAggregator::new(client),
        })
    }

    fn create_key<T: StaticResource>(
        namespace: Option<String>,
        name: &str,
    ) -> Result<String, Error> {
        if T::is_cluster_scoped() {
            Ok(format!(
                "{BASE_PATH}/{}/{}/{}",
                T::group(),
                T::plural(),
                name
            ))
        } else {
            let ns =
                namespace.ok_or_else(|| Error::FieldMissing("metadata.namespace".to_string()))?;
            Ok(format!(
                "{BASE_PATH}/{}/{}/{}/{}",
                T::group(),
                T::plural(),
                ns,
                name
            ))
        }
    }

    fn create_watch_key<T: StaticResource>(namespace: Option<String>) -> String {
        if let Some(ns) = namespace {
            format!("{BASE_PATH}/{}/{}/{ns}/", T::group(), T::plural())
        } else {
            format!("{BASE_PATH}/{}/{}/", T::group(), T::plural())
        }
    }

    fn create_custom_key(group: &str, plural: &str, namespace: Option<&str>, name: &str) -> String {
        match namespace {
            Some(ns) => format!("{BASE_PATH}/{group}/{plural}/{ns}/{name}"),
            None => format!("{BASE_PATH}/{group}/{plural}/{name}"),
        }
    }

    fn create_custom_watch_key(group: &str, plural: &str, namespace: Option<&str>) -> String {
        match namespace {
            Some(ns) => format!("{BASE_PATH}/{group}/{plural}/{ns}/"),
            None => format!("{BASE_PATH}/{group}/{plural}/"),
        }
    }

    pub fn prepare_put<T: StaticSerializable + ObjectMetaResource>(
        &self,
        value: &T,
    ) -> Result<PutRequest, Error> {
        Self::prepare_put_inner(value)
    }

    pub async fn put_many(&self, requests: Vec<PutRequest>) -> Result<i64, Error> {
        if requests.is_empty() {
            return Ok(0);
        }

        info!("Put multiple resources: count = {}", requests.len());
        let first_expected_revision = requests
            .iter()
            .find_map(|request| request.expected_revision)
            .unwrap_or(0);
        let compares = requests
            .iter()
            .filter_map(|request| {
                request.expected_revision.map(|revision| {
                    Compare::mod_revision(request.key.as_str(), CompareOp::Equal, revision)
                })
            })
            .collect::<Vec<_>>();
        let puts = requests
            .iter()
            .map(|request| TxnOp::put(request.key.as_str(), request.bytes.clone(), None))
            .collect::<Vec<_>>();

        let mut txn = Txn::new();
        if !compares.is_empty() {
            txn = txn.when(compares);
        }
        txn = txn.and_then(puts);
        if requests
            .iter()
            .any(|request| request.expected_revision.is_some())
        {
            let conflict_reads = requests
                .iter()
                .filter(|request| request.expected_revision.is_some())
                .map(|request| TxnOp::get(request.key.as_str(), None))
                .collect::<Vec<_>>();
            txn = txn.or_else(conflict_reads);
        }

        let mut client = self.etcd.clone();
        let response = client.txn(txn).await?;
        if !response.succeeded() {
            return Err(Error::OptimisticLockFailed(first_expected_revision));
        }

        Ok(response
            .header()
            .map(|header| header.revision())
            .unwrap_or(0))
    }

    pub async fn put<T: StaticSerializable + ObjectMetaResource>(
        &self,
        value: T,
    ) -> Result<ContentData<T>, Error> {
        info!("Put resource");
        let request = Self::prepare_put_inner(&value)?;
        let revision = self.put_many(vec![request]).await?;

        Ok(ContentData {
            data: value,
            revision,
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
        let key = Self::create_key::<T>(meta.namespace.clone(), name)?;
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
        let key = Self::create_key::<T>(namespace, name)?;
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
        Ok(self.list_with_revision::<T>(namespace, limit).await?.0)
    }

    pub async fn list_with_revision<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        limit: Option<usize>,
    ) -> Result<(Vec<ContentData<T>>, i64), Error> {
        let key = Self::create_watch_key::<T>(namespace);
        info!("List resources: key = {key}");
        let mut client = self.etcd.clone();
        let mut options = GetOptions::default().with_prefix();
        if let Some(limit) = limit {
            options = options.with_limit(limit as i64);
        }

        let res = client.get(key, Some(options)).await?;
        let header_revision = res.header().map(|h| h.revision()).unwrap_or(0);
        let mut data = Vec::new();
        for kv in res.kvs() {
            let value = T::deserialize(kv.value())?;
            data.push(ContentData {
                data: value,
                revision: kv.mod_revision(),
            });
        }
        Ok((data, header_revision))
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

    pub async fn watch_from_revision<T: StaticSerializable>(
        &self,
        start_revision: Option<i64>,
        namespace: Option<String>,
    ) -> Result<WatchReceiver, Error> {
        let key = Self::create_watch_key::<T>(namespace);
        info!("Watch resources from revision: key = {key}, start = {start_revision:?}");
        self.watch_mux
            .get_from_start_revision(&key, start_revision)
            .await
    }

    pub async fn delete<T: StaticSerializable>(
        &self,
        namespace: Option<String>,
        name: &str,
    ) -> Result<Option<ContentData<T>>, Error> {
        let key = Self::create_key::<T>(namespace, name)?;
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

    pub async fn put_custom(
        &self,
        group: &str,
        plural: &str,
        namespace: Option<&str>,
        name: &str,
        envelope: &CustomResourceObject,
        expected_revision: Option<i64>,
    ) -> Result<i64, Error> {
        let key = Self::create_custom_key(group, plural, namespace, name);
        let bytes = envelope.serialize()?;
        debug!(
            "Put custom resource key = {key}, value = {} bytes",
            bytes.len()
        );

        let request = PutRequest {
            key,
            bytes,
            expected_revision,
        };
        self.put_many(vec![request]).await
    }

    pub async fn get_custom(
        &self,
        group: &str,
        plural: &str,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Option<ContentData<CustomResourceObject>>, Error> {
        let key = Self::create_custom_key(group, plural, namespace, name);
        info!("Get custom resource: key = {key}");

        let mut client = self.etcd.clone();
        let res = client.get(key, None).await?;

        let Some(kv) = res.kvs().first() else {
            return Ok(None);
        };
        let value = CustomResourceObject::deserialize(kv.value())?;
        Ok(Some(ContentData {
            data: value,
            revision: kv.mod_revision(),
        }))
    }

    pub async fn list_custom(
        &self,
        group: &str,
        plural: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<ContentData<CustomResourceObject>>, Error> {
        let key = Self::create_custom_watch_key(group, plural, namespace);
        info!("List custom resources: key = {key}");

        let mut client = self.etcd.clone();
        let res = client
            .get(key, Some(GetOptions::default().with_prefix()))
            .await?;
        let mut data = Vec::new();
        for kv in res.kvs() {
            let value = CustomResourceObject::deserialize(kv.value())?;
            data.push(ContentData {
                data: value,
                revision: kv.mod_revision(),
            });
        }
        Ok(data)
    }

    pub async fn delete_custom(
        &self,
        group: &str,
        plural: &str,
        namespace: Option<&str>,
        name: &str,
        expected_revision: Option<i64>,
    ) -> Result<bool, Error> {
        let key = Self::create_custom_key(group, plural, namespace, name);
        info!("Delete custom resource: key = {key}");

        let mut client = self.etcd.clone();
        let Some(expected_revision) = expected_revision else {
            let response = client.delete(key, None).await?;
            return Ok(response.deleted() > 0);
        };

        let txn = Txn::new()
            .when(vec![Compare::mod_revision(
                key.as_str(),
                CompareOp::Equal,
                expected_revision,
            )])
            .and_then(vec![TxnOp::delete(key.as_str(), None)]);
        let response = client.txn(txn).await?;
        if !response.succeeded() {
            return Err(Error::OptimisticLockFailed(expected_revision));
        }
        Ok(response
            .op_responses()
            .into_iter()
            .filter_map(|op| match op {
                TxnOpResponse::Delete(res) => Some(res.deleted() > 0),
                _ => None,
            })
            .next()
            .unwrap_or(false))
    }

    pub async fn delete_custom_collection(&self, group: &str, plural: &str) -> Result<i64, Error> {
        let key = Self::create_custom_watch_key(group, plural, None);
        info!("Delete custom resource collection: key = {key}");

        let mut client = self.etcd.clone();
        let response = client
            .delete(key, Some(DeleteOptions::default().with_prefix()))
            .await?;
        Ok(response.deleted())
    }

    pub async fn watch_custom(
        &self,
        group: &str,
        plural: &str,
        namespace: Option<&str>,
        start_revision: Option<i64>,
    ) -> Result<WatchReceiver, Error> {
        let key = Self::create_custom_watch_key(group, plural, namespace);
        info!("Watch custom resources: key = {key}");
        self.watch_mux
            .get_from_start_revision(&key, start_revision)
            .await
    }
}

fn build_connect_options(tls_config: EtcdTlsConfig) -> Result<ConnectOptions, Error> {
    let ca_cert = std::fs::read(&tls_config.ca_cert_path)?;
    let cert = std::fs::read(&tls_config.cert_path)?;
    let key = std::fs::read(&tls_config.key_path)?;
    let mut tls = TlsOptions::new()
        .ca_certificate(Certificate::from_pem(ca_cert))
        .identity(Identity::from_pem(cert, key));

    if let Some(domain_name) = tls_config.domain_name {
        tls = tls.domain_name(domain_name);
    }

    Ok(ConnectOptions::default().with_tls(tls))
}

#[cfg(test)]
mod tests {
    use super::ContentData;
    use super::ResourceStore;
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::core::v1::Ship;
    use tugboat_resources::manifests::meta::v1::{CustomResourceObject, ObjectMeta, TypeMeta};

    #[test]
    fn apply_revision_preserves_generation() {
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("demo".to_string()),
                namespace: Some("default".to_string()),
                generation: Some(7),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ship = ContentData {
            data: ship,
            revision: 42,
        }
        .apply_revision();

        assert_eq!(
            ship.object_meta()
                .as_ref()
                .and_then(|meta| meta.resource_version.as_deref()),
            Some("42")
        );
        assert_eq!(
            ship.object_meta().as_ref().and_then(|meta| meta.generation),
            Some(7)
        );
    }

    #[test]
    fn custom_resource_keys_follow_registry_layout() {
        assert_eq!(
            ResourceStore::create_custom_key("example.com", "widgets", Some("default"), "demo"),
            "/tugboat/registry/example.com/widgets/default/demo"
        );
        assert_eq!(
            ResourceStore::create_custom_key("example.com", "widgets", None, "demo"),
            "/tugboat/registry/example.com/widgets/demo"
        );
        assert_eq!(
            ResourceStore::create_custom_watch_key("example.com", "widgets", Some("default")),
            "/tugboat/registry/example.com/widgets/default/"
        );
        assert_eq!(
            ResourceStore::create_custom_watch_key("example.com", "widgets", None),
            "/tugboat/registry/example.com/widgets/"
        );
    }

    #[tokio::test]
    async fn custom_resource_crud_round_trip_when_etcd_endpoint_is_configured() {
        let Ok(endpoint) =
            std::env::var("TUGBOAT_TEST_ETCD_ENDPOINT").or_else(|_| std::env::var("ETCD_ENDPOINT"))
        else {
            return;
        };

        let store = ResourceStore::new_insecure(&[endpoint])
            .await
            .expect("connect to etcd");
        let group = "test.example.com";
        let plural = "widgets";
        let namespace = format!("store-test-{}", std::process::id());
        let name = "demo";
        let raw_json = br#"{"apiVersion":"test.example.com/v1","kind":"Widget","metadata":{"name":"demo"},"spec":{"size":3}}"#.to_vec();
        let envelope = CustomResourceObject {
            type_meta: Some(TypeMeta {
                api_version: Some("test.example.com/v1".to_string()),
                kind: Some("Widget".to_string()),
            }),
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.clone()),
                ..Default::default()
            }),
            raw_json: raw_json.clone(),
        };

        let _ = store
            .delete_custom(group, plural, Some(namespace.as_str()), name, None)
            .await;
        let revision = store
            .put_custom(
                group,
                plural,
                Some(namespace.as_str()),
                name,
                &envelope,
                None,
            )
            .await
            .expect("put custom resource");
        assert!(revision > 0);

        let fetched = store
            .get_custom(group, plural, Some(namespace.as_str()), name)
            .await
            .expect("get custom resource")
            .expect("custom resource should exist");
        assert_eq!(fetched.data.raw_json, raw_json);

        let listed = store
            .list_custom(group, plural, Some(namespace.as_str()))
            .await
            .expect("list custom resources");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].data.raw_json, raw_json);

        assert!(
            store
                .delete_custom(group, plural, Some(namespace.as_str()), name, None)
                .await
                .expect("delete custom resource")
        );
        assert!(
            store
                .get_custom(group, plural, Some(namespace.as_str()), name)
                .await
                .expect("get deleted custom resource")
                .is_none()
        );
    }
}
