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

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::crd_schema::{CompiledSchema, compile_schema, parse_schema};
use thiserror::Error;
use tokio::time::sleep;
use tracing::{error, info, warn};
use tugboat_resource_store::ResourceStore;
use tugboat_resource_store::serializer::Serializable;
use tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinition;

pub use tugboat_resources::manifests::apiextensions::v1::RESERVED_GROUPS;

#[derive(Clone, Debug, PartialEq)]
pub struct CrdEntry {
    pub group: String,
    pub plural: String,
    pub singular: String,
    pub kind: String,
    pub list_kind: String,
    pub scope: CrdScope,
    pub version: CrdVersionInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrdScope {
    Cluster,
    Namespaced,
}

#[derive(Clone, Debug)]
pub struct CrdVersionInfo {
    pub name: String,
    pub served: bool,
    pub storage: bool,
    pub schema_json: Option<Arc<serde_json::Value>>,
    pub compiled_schema: Option<Arc<CompiledSchema>>,
    pub status_subresource: bool,
}

impl PartialEq for CrdVersionInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.served == other.served
            && self.storage == other.storage
            && self.schema_json == other.schema_json
            && self.status_subresource == other.status_subresource
    }
}

#[derive(Debug, Error)]
pub enum CrdRegistryError {
    #[error("CRD group '{0}' is reserved for built-in resources")]
    ReservedGroup(String),
}

#[derive(Debug, Error)]
pub enum CrdRegistrySyncError {
    #[error("resource store error: {0}")]
    Store(#[from] tugboat_resource_store::error::Error),
    #[error("CRD registry error: {0}")]
    Registry(#[from] CrdRegistryError),
}

#[derive(Default)]
pub struct CrdRegistry {
    inner: RwLock<CrdRegistryInner>,
}

#[derive(Default)]
struct CrdRegistryInner {
    by_group_version_plural: HashMap<(String, String, String), CrdEntry>,
}

impl CrdRegistry {
    pub fn lookup(&self, group: &str, version: &str, plural: &str) -> Option<CrdEntry> {
        self.inner
            .read()
            .expect("CRD registry lock poisoned")
            .by_group_version_plural
            .get(&(group.to_string(), version.to_string(), plural.to_string()))
            .cloned()
    }

    pub fn list_all(&self) -> Vec<CrdEntry> {
        self.inner
            .read()
            .expect("CRD registry lock poisoned")
            .by_group_version_plural
            .values()
            .cloned()
            .collect()
    }

    pub fn is_registered(&self, group: &str, version: &str, plural: &str) -> bool {
        self.lookup(group, version, plural).is_some()
    }

    pub fn upsert(&self, entry: CrdEntry) -> Result<(), CrdRegistryError> {
        ensure_group_allowed(&entry.group)?;
        let key = entry.key();
        self.inner
            .write()
            .expect("CRD registry lock poisoned")
            .by_group_version_plural
            .insert(key, entry);
        Ok(())
    }

    pub fn remove(&self, group: &str, version: &str, plural: &str) {
        self.inner
            .write()
            .expect("CRD registry lock poisoned")
            .by_group_version_plural
            .remove(&(group.to_string(), version.to_string(), plural.to_string()));
    }

    pub fn remove_crd(&self, crd: &CustomResourceDefinition) {
        let Some(spec) = crd.spec.as_ref() else {
            return;
        };
        let Some(names) = spec.names.as_ref() else {
            return;
        };
        for version in &spec.versions {
            self.remove(&spec.group, &version.name, &names.plural);
        }
    }

    pub fn replace_all(&self, entries: Vec<CrdEntry>) -> Result<(), CrdRegistryError> {
        let mut by_group_version_plural = HashMap::with_capacity(entries.len());
        for entry in entries {
            ensure_group_allowed(&entry.group)?;
            by_group_version_plural.insert(entry.key(), entry);
        }
        self.inner
            .write()
            .expect("CRD registry lock poisoned")
            .by_group_version_plural = by_group_version_plural;
        Ok(())
    }

    pub fn from_crd(crd: &CustomResourceDefinition) -> Option<CrdEntry> {
        let spec = crd.spec.as_ref()?;
        let names = spec.names.as_ref()?;
        let version = spec.versions.first()?;
        let scope = match spec.scope.as_str() {
            "Cluster" => CrdScope::Cluster,
            "Namespaced" => CrdScope::Namespaced,
            _ => return None,
        };
        let (schema_json, compiled_schema) = match version.schema.as_ref() {
            Some(schema) if !schema.open_api_v3_schema.trim().is_empty() => {
                let schema_json = Arc::new(parse_schema(&schema.open_api_v3_schema).ok()?);
                let compiled_schema = Arc::new(compile_schema(&schema_json).ok()?);
                (Some(schema_json), Some(compiled_schema))
            }
            _ => (None, None),
        };
        let list_kind = if names.list_kind.is_empty() {
            format!("{}List", names.kind)
        } else {
            names.list_kind.clone()
        };

        Some(CrdEntry {
            group: spec.group.clone(),
            plural: names.plural.clone(),
            singular: names.singular.clone(),
            kind: names.kind.clone(),
            list_kind,
            scope,
            version: CrdVersionInfo {
                name: version.name.clone(),
                served: version.served,
                storage: version.storage,
                schema_json,
                compiled_schema,
                status_subresource: version
                    .subresources
                    .as_ref()
                    .and_then(|subresources| subresources.status.as_ref())
                    .is_some(),
            },
        })
    }
}

impl CrdEntry {
    fn key(&self) -> (String, String, String) {
        (
            self.group.clone(),
            self.version.name.clone(),
            self.plural.clone(),
        )
    }
}

pub async fn load_crds_into_registry(
    store: &ResourceStore,
    registry: &CrdRegistry,
) -> Result<i64, CrdRegistrySyncError> {
    let (crds, revision) = store
        .list_with_revision::<CustomResourceDefinition>(None, None)
        .await?;
    let entries = crds
        .iter()
        .filter_map(|crd| CrdRegistry::from_crd(&crd.data))
        .collect::<Vec<_>>();
    registry.replace_all(entries)?;
    info!(
        "Loaded {} CRDs into registry at revision {revision}",
        registry.list_all().len()
    );
    Ok(revision)
}

pub async fn run_crd_watcher(store: Arc<ResourceStore>, registry: Arc<CrdRegistry>) {
    loop {
        let revision = match load_crds_into_registry(&store, &registry).await {
            Ok(revision) => revision,
            Err(err) => {
                error!("Failed to load CRDs into registry: {err}");
                sleep(Duration::from_millis(500)).await;
                continue;
            }
        };

        // Start the watch immediately after the snapshot's revision so any events
        // between the list and the watch subscription are replayed deterministically.
        let start_revision = Some(revision.saturating_add(1));
        let mut watch = match store
            .watch_from_revision::<CustomResourceDefinition>(start_revision, None)
            .await
        {
            Ok(watch) => watch,
            Err(err) => {
                error!("Failed to start CRD watcher: {err}");
                sleep(Duration::from_millis(500)).await;
                continue;
            }
        };

        loop {
            if watch.changed().await.is_err() {
                warn!("CRD watcher channel closed; resynchronizing registry");
                break;
            }
            let events = watch.borrow_and_update().clone();
            for event in events {
                apply_watch_event(&registry, event);
            }
        }
    }
}

fn apply_watch_event(registry: &CrdRegistry, event: tugboat_resource_store::watch::WatchEvent) {
    match event {
        tugboat_resource_store::watch::WatchEvent::Added(kv)
        | tugboat_resource_store::watch::WatchEvent::Modified(kv) => {
            match CustomResourceDefinition::deserialize(kv.value.as_slice()) {
                Ok(crd) => match CrdRegistry::from_crd(&crd) {
                    Some(entry) => {
                        if let Err(err) = registry.upsert(entry) {
                            warn!("Ignoring CRD registry update: {err}");
                        }
                    }
                    None => warn!("Ignoring invalid CRD watch event"),
                },
                Err(err) => warn!("Failed to deserialize CRD watch event: {err}"),
            }
        }
        tugboat_resource_store::watch::WatchEvent::Deleted(kv) => {
            match CustomResourceDefinition::deserialize(kv.value.as_slice()) {
                Ok(crd) => registry.remove_crd(&crd),
                Err(err) => warn!("Failed to deserialize deleted CRD watch event: {err}"),
            }
        }
    }
}

fn ensure_group_allowed(group: &str) -> Result<(), CrdRegistryError> {
    if RESERVED_GROUPS.contains(&group) {
        Err(CrdRegistryError::ReservedGroup(group.to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CrdRegistry, CrdRegistryError, CrdScope};
    use tugboat_resources::manifests::apiextensions::v1::{
        CustomResourceDefinition, CustomResourceDefinitionNames, CustomResourceDefinitionSpec,
        CustomResourceDefinitionVersion, CustomResourceSubresourceStatus,
        CustomResourceSubresources, CustomResourceValidation,
    };

    fn crd() -> CustomResourceDefinition {
        CustomResourceDefinition {
            spec: Some(CustomResourceDefinitionSpec {
                group: "example.com".to_string(),
                names: Some(CustomResourceDefinitionNames {
                    plural: "widgets".to_string(),
                    singular: "widget".to_string(),
                    kind: "Widget".to_string(),
                    list_kind: "WidgetList".to_string(),
                }),
                scope: "Namespaced".to_string(),
                versions: vec![CustomResourceDefinitionVersion {
                    name: "v1".to_string(),
                    served: true,
                    storage: true,
                    schema: Some(CustomResourceValidation {
                        open_api_v3_schema:
                            r#"{"type":"object","properties":{"spec":{"type":"object"}}}"#
                                .to_string(),
                    }),
                    subresources: Some(CustomResourceSubresources {
                        status: Some(CustomResourceSubresourceStatus {}),
                    }),
                }],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn from_crd_extracts_registry_entry() {
        let entry = CrdRegistry::from_crd(&crd()).expect("valid CRD should convert");

        assert_eq!(entry.group, "example.com");
        assert_eq!(entry.plural, "widgets");
        assert_eq!(entry.singular, "widget");
        assert_eq!(entry.kind, "Widget");
        assert_eq!(entry.list_kind, "WidgetList");
        assert_eq!(entry.scope, CrdScope::Namespaced);
        assert_eq!(entry.version.name, "v1");
        assert!(entry.version.served);
        assert!(entry.version.storage);
        assert!(entry.version.status_subresource);
        assert_eq!(
            entry
                .version
                .schema_json
                .as_ref()
                .and_then(|schema| schema.get("type"))
                .and_then(|value| value.as_str()),
            Some("object")
        );
    }

    #[test]
    fn from_crd_rejects_invalid_schema_json() {
        let mut crd = crd();
        crd.spec.as_mut().unwrap().versions[0].schema = Some(CustomResourceValidation {
            open_api_v3_schema: "{".to_string(),
        });

        assert!(CrdRegistry::from_crd(&crd).is_none());
    }

    #[test]
    fn upsert_rejects_reserved_groups() {
        let mut entry = CrdRegistry::from_crd(&crd()).expect("valid CRD should convert");
        entry.group = "core".to_string();

        let err = CrdRegistry::default()
            .upsert(entry)
            .expect_err("reserved group should be rejected");
        assert!(matches!(err, CrdRegistryError::ReservedGroup(group) if group == "core"));
    }

    #[test]
    fn upsert_lookup_and_remove_crd_keep_registry_in_sync() {
        let crd = crd();
        let registry = CrdRegistry::default();
        let entry = CrdRegistry::from_crd(&crd).expect("valid CRD should convert");

        registry.upsert(entry).expect("upsert should succeed");
        assert!(registry.is_registered("example.com", "v1", "widgets"));
        assert_eq!(
            registry
                .lookup("example.com", "v1", "widgets")
                .map(|entry| entry.kind),
            Some("Widget".to_string())
        );

        registry.remove_crd(&crd);
        assert!(!registry.is_registered("example.com", "v1", "widgets"));
    }

    #[test]
    fn replace_all_rejects_reserved_groups_without_partial_update() {
        let registry = CrdRegistry::default();
        let valid = CrdRegistry::from_crd(&crd()).expect("valid CRD should convert");
        registry.upsert(valid).expect("upsert should succeed");

        let mut reserved = CrdRegistry::from_crd(&crd()).expect("valid CRD should convert");
        reserved.group = "apps".to_string();
        let err = registry
            .replace_all(vec![reserved])
            .expect_err("reserved group should fail replace_all");

        assert!(matches!(err, CrdRegistryError::ReservedGroup(group) if group == "apps"));
        assert!(registry.is_registered("example.com", "v1", "widgets"));
    }
}
