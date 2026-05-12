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

use crate::data::{ModifyResponse, ReadResponse, ResourceList, StatusResponse};
use crate::endpoints::ListQuery;
use crate::endpoints::selector::FilterBySelector;
use crate::endpoints::watch_utils::watch;
use crate::operator::ApiOperator;
use actix_web::HttpResponse;
use actix_web::web::Data;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tugboat_resource_store::serializer::StaticSerializable;
use tugboat_resources::manifests::core::v1::Namespace;
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_resources::validators::Validatable;
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta, StaticResource};

pub(crate) async fn create_cluster<T>(
    object: T,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: Resource + StaticSerializable + ObjectMetaResource + SetTypeMeta + Clone + Validatable,
{
    let object_meta = crate::extract_object_meta!(object);
    crate::check_namespace_absent!(object_meta);
    validate_resource(&object)?;

    crate::create_object!(operator, object_meta, object, T::type_meta())
}

pub(crate) async fn create_namespaced<T>(
    object: T,
    namespace: String,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: Resource + StaticSerializable + ObjectMetaResource + SetTypeMeta + Clone + Validatable,
{
    ensure_namespace_exists(&operator, &namespace).await?;
    let object_meta = crate::extract_object_meta!(object);
    let object_meta = operator.apply_namespace(object_meta, namespace);
    validate_resource(&object)?;

    crate::create_object!(operator, object_meta, object, T::type_meta())
}

pub(crate) async fn list_resources<T>(
    operator: &ApiOperator,
    query: ListQuery,
    namespace: Option<String>,
) -> Result<HttpResponse, Box<StatusResponse>>
where
    T: 'static + StaticSerializable + ObjectMetaResource + Serialize,
{
    if let Some(opts) = query.watch {
        watch::<T>(
            operator,
            opts,
            query.to_field_selector()?,
            query.to_label_selector()?,
            query.resource_version,
            namespace,
        )
        .await
    } else {
        let field_selector = query.to_field_selector()?;
        let label_selector = query.to_label_selector()?;
        let resources = operator
            .store
            .list::<T>(namespace, None)
            .await
            .map_err(|e| Box::new(e.into()))?;
        Ok(ResourceList::from_serializable(
            resources
                .into_iter()
                .map(|data| data.apply_revision())
                .filter_by_selector(field_selector, label_selector),
        )?
        .into())
    }
}

pub(crate) async fn read_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
) -> Result<ReadResponse<T>, Box<StatusResponse>>
where
    T: StaticSerializable + ObjectMetaResource + StaticResource + Serialize,
{
    let resource = operator
        .store
        .get::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    match resource {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        ))),
    }
}

pub(crate) async fn delete_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
) -> Result<ReadResponse<T>, Box<StatusResponse>>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq
        + Clone,
{
    let current = operator
        .store
        .get::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )));
    };
    let current = current.apply_revision();

    if current.has_finalizers() {
        let mut pending_delete = current.clone();
        pending_delete.mark_for_deletion(Time::now());
        let pending_delete = if current != pending_delete {
            operator
                .store
                .put(pending_delete)
                .await
                .map_err(|e| Box::new(e.into()))?
                .apply_revision()
        } else {
            pending_delete
        };

        return Ok(ReadResponse::new(pending_delete));
    }

    let resource = operator
        .store
        .delete::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    match resource {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        ))),
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ReplaceOptions {
    pub(crate) preserve_status: bool,
    pub(crate) use_client_resource_version: bool,
    pub(crate) update_generation: bool,
}

pub(crate) struct ResourceUpdater<'a, T> {
    current: &'a T,
    options: ReplaceOptions,
}

impl<'a, T> ResourceUpdater<'a, T>
where
    T: ObjectMetaResource + StaticResource + Serialize + DeserializeOwned,
{
    pub(crate) fn new(current: &'a T, options: ReplaceOptions) -> Self {
        Self { current, options }
    }

    pub(crate) fn apply_patch(
        self,
        patch: serde_json::Map<String, serde_json::Value>,
    ) -> Result<T, Box<StatusResponse>> {
        let current_value = serde_json::to_value(self.current).map_err(|e| Box::new(e.into()))?;
        let current_obj = to_object(current_value, "current resource")?;

        let patch_metadata = patch.get("metadata").cloned();

        let mut merged_value = serde_json::Value::Object(current_obj.clone());
        rfc7396_merge_patch(&mut merged_value, &serde_json::Value::Object(patch));
        let mut merged_obj = to_object(merged_value, "patched resource")?;
        let content_changed = generation_tracked_fields_changed(&current_obj, &merged_obj);

        // Restore fields that must be preserved from current
        if let Some(v) = current_obj.get("apiVersion") {
            merged_obj.insert("apiVersion".to_string(), v.clone());
        }
        if let Some(v) = current_obj.get("kind") {
            merged_obj.insert("kind".to_string(), v.clone());
        }

        if self.options.preserve_status {
            if let Some(status) = current_obj.get("status") {
                merged_obj.insert("status".to_string(), status.clone());
            } else {
                merged_obj.remove("status");
            }
        }

        let mut updated: T = serde_json::from_value(serde_json::Value::Object(merged_obj))
            .map_err(|e| Box::new(e.into()))?;

        self.enforce_metadata(&mut updated, patch_metadata.as_ref(), content_changed);

        Ok(updated)
    }

    pub(crate) fn apply_replacement(self, replacement: &T) -> Result<T, Box<StatusResponse>> {
        let current_value = serde_json::to_value(self.current).map_err(|e| Box::new(e.into()))?;
        let current_obj = to_object(current_value, "current resource")?;
        let current_generation_fields = generation_tracked_fields(&current_obj);

        let replacement_value =
            serde_json::to_value(replacement).map_err(|e| Box::new(e.into()))?;
        let mut replacement_obj = to_object(replacement_value, "replacement resource")?;

        let patch_metadata = replacement_obj.get("metadata").cloned();

        // Restore fields that must be preserved from current
        if let Some(v) = current_obj.get("apiVersion") {
            replacement_obj.insert("apiVersion".to_string(), v.clone());
        }
        if let Some(v) = current_obj.get("kind") {
            replacement_obj.insert("kind".to_string(), v.clone());
        }

        if self.options.preserve_status {
            if let Some(status) = current_obj.get("status") {
                replacement_obj.insert("status".to_string(), status.clone());
            } else {
                replacement_obj.remove("status");
            }
        }

        let content_changed =
            current_generation_fields != generation_tracked_fields(&replacement_obj);
        let mut updated: T = serde_json::from_value(serde_json::Value::Object(replacement_obj))
            .map_err(|e| Box::new(e.into()))?;

        self.enforce_metadata(&mut updated, patch_metadata.as_ref(), content_changed);

        Ok(updated)
    }

    pub(crate) fn apply_status_update(
        self,
        status: serde_json::Value,
    ) -> Result<T, Box<StatusResponse>> {
        let current_value = serde_json::to_value(self.current).map_err(|e| Box::new(e.into()))?;
        let mut current_obj = to_object(current_value, "current resource")?;

        current_obj.insert("status".to_string(), status);

        let mut updated: T = serde_json::from_value(serde_json::Value::Object(current_obj))
            .map_err(|e| Box::new(e.into()))?;

        // For status updates, we always preserve the current metadata.
        self.enforce_metadata(&mut updated, None, false);

        Ok(updated)
    }

    fn enforce_metadata(
        &self,
        updated: &mut T,
        patch_metadata: Option<&serde_json::Value>,
        content_changed: bool,
    ) {
        if let (Some(current_meta), Some(updated_meta)) =
            (self.current.object_meta(), updated.object_meta_mut())
        {
            updated_meta.name = current_meta.name.clone();
            updated_meta.namespace = current_meta.namespace.clone();
            updated_meta.uid = current_meta.uid.clone();
            updated_meta.generation = if self.options.update_generation && content_changed {
                next_generation(current_meta.generation)
            } else {
                current_meta.generation
            };
            updated_meta.creation_timestamp = current_meta.creation_timestamp;
            updated_meta.deletion_timestamp = current_meta.deletion_timestamp;

            if self.options.use_client_resource_version {
                let client_rv = patch_metadata
                    .and_then(|m| m.get("resourceVersion"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                updated_meta.resource_version = client_rv;
            } else {
                updated_meta.resource_version = current_meta.resource_version.clone();
            }
        }
    }
}

pub(crate) fn bump_generation<T>(resource: &mut T)
where
    T: ObjectMetaResource,
{
    if let Some(meta) = resource.object_meta_mut().as_mut() {
        meta.generation = next_generation(meta.generation);
    }
}

pub(crate) async fn replace_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
    replacement: T,
    options: ReplaceOptions,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq
        + Validatable,
{
    let current = operator
        .store
        .get::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )));
    };
    let current = current.apply_revision();
    validate_resource_name(&replacement, &name)?;
    let replaced = ResourceUpdater::new(&current, options).apply_replacement(&replacement)?;
    validate_resource(&replaced)?;

    let replaced = if current != replaced {
        operator
            .store
            .put(replaced)
            .await
            .map_err(|e| Box::new(e.into()))?
            .apply_revision()
    } else {
        replaced
    };
    Ok(ModifyResponse::Updated(replaced))
}

pub(crate) async fn patch_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
    patch: serde_json::Map<String, serde_json::Value>,
    options: ReplaceOptions,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq
        + Validatable,
{
    validate_patch_name(&patch, &name)?;
    let current = operator
        .store
        .get::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )));
    };
    let current = current.apply_revision();
    let patched = ResourceUpdater::new(&current, options).apply_patch(patch)?;
    validate_resource(&patched)?;

    let patched = if current != patched {
        operator
            .store
            .put(patched)
            .await
            .map_err(|e| Box::new(e.into()))?
            .apply_revision()
    } else {
        patched
    };
    Ok(ModifyResponse::Updated(patched))
}

pub(crate) async fn status_patch_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq
        + Validatable,
{
    if !patch.keys().all(|key| key == "status") {
        return Err(Box::new(StatusResponse::bad_request(
            "PATCH /status must contain only status field.",
            None,
        )));
    }

    let current = operator
        .store
        .get::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )));
    };
    let current = current.apply_revision();

    let options = ReplaceOptions {
        preserve_status: false,
        ..Default::default()
    };
    let patched = ResourceUpdater::new(&current, options).apply_patch(patch)?;
    validate_resource(&patched)?;

    let patched = if current != patched {
        operator
            .store
            .put(patched)
            .await
            .map_err(|e| Box::new(e.into()))?
            .apply_revision()
    } else {
        patched
    };
    Ok(ModifyResponse::Updated(patched))
}

pub(crate) async fn status_replace_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
    replacement: T,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq
        + Validatable,
{
    let current = operator
        .store
        .get::<T>(namespace.clone(), &name)
        .await
        .map_err(|e| Box::new(e.into()))?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )));
    };
    let current = current.apply_revision();

    let replacement_value = serde_json::to_value(replacement).map_err(|e| Box::new(e.into()))?;
    let status = replacement_value
        .as_object()
        .and_then(|obj| obj.get("status").cloned())
        .unwrap_or(serde_json::Value::Null);

    let replaced =
        ResourceUpdater::new(&current, ReplaceOptions::default()).apply_status_update(status)?;
    validate_resource(&replaced)?;

    let replaced = if current != replaced {
        operator
            .store
            .put(replaced)
            .await
            .map_err(|e| Box::new(e.into()))?
            .apply_revision()
    } else {
        replaced
    };
    Ok(ModifyResponse::Updated(replaced))
}

fn to_object(
    value: serde_json::Value,
    context: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, Box<StatusResponse>> {
    match value {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(Box::new(StatusResponse::internal_error(
            format!("{context} is not a JSON object"),
            None,
        ))),
    }
}

fn resource_identity(namespace: Option<&str>, name: &str) -> serde_json::Value {
    if let Some(namespace) = namespace {
        serde_json::json!({ "namespace": namespace, "name": name })
    } else {
        serde_json::json!({ "name": name })
    }
}

async fn ensure_namespace_exists(
    operator: &ApiOperator,
    namespace: &str,
) -> Result<(), Box<StatusResponse>> {
    let found = operator
        .store
        .get::<Namespace>(None, namespace)
        .await
        .map_err(|e| Box::new(e.into()))?;
    if found.is_some() {
        Ok(())
    } else {
        Err(Box::new(StatusResponse::not_found(
            format!("namespaces \"{namespace}\" not found"),
            Some(serde_json::json!({ "name": namespace })),
        )))
    }
}

pub(crate) fn validate_resource<T>(resource: &T) -> Result<(), Box<StatusResponse>>
where
    T: StaticResource + Validatable,
{
    if resource.validate() {
        Ok(())
    } else {
        Err(Box::new(StatusResponse::bad_request(
            format!("Invalid {} resource", T::kind()),
            None,
        )))
    }
}

pub(crate) fn validate_resource_name<T>(
    resource: &T,
    expected_name: &str,
) -> Result<(), Box<StatusResponse>>
where
    T: StaticResource + ObjectMetaResource,
{
    if let Some(actual_name) = resource.name()
        && actual_name != expected_name
    {
        return Err(Box::new(StatusResponse::bad_request(
            format!(
                "metadata.name must match resource name in URL: expected \"{expected_name}\", got \"{actual_name}\""
            ),
            Some(serde_json::json!({
                "name": expected_name,
                "providedName": actual_name,
            })),
        )));
    }

    Ok(())
}

fn validate_patch_name(
    patch: &serde_json::Map<String, serde_json::Value>,
    expected_name: &str,
) -> Result<(), Box<StatusResponse>> {
    let actual_name = patch
        .get("metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(|name| name.as_str());

    if let Some(actual_name) = actual_name
        && actual_name != expected_name
    {
        return Err(Box::new(StatusResponse::bad_request(
            format!(
                "metadata.name must match resource name in URL: expected \"{expected_name}\", got \"{actual_name}\""
            ),
            Some(serde_json::json!({
                "name": expected_name,
                "providedName": actual_name,
            })),
        )));
    }
    Ok(())
}

fn generation_tracked_fields(
    object: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    object
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "metadata" | "apiVersion" | "kind" | "status"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn generation_tracked_fields_changed(
    current: &serde_json::Map<String, serde_json::Value>,
    updated: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    generation_tracked_fields(current) != generation_tracked_fields(updated)
}

fn next_generation(current: Option<i64>) -> Option<i64> {
    Some(current.unwrap_or(0).max(0).saturating_add(1))
}

/// Applies a JSON merge patch (RFC 7396) to `target` in place.
/// Objects are merged recursively; all other types (including arrays) are replaced.
fn rfc7396_merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match patch {
        serde_json::Value::Object(patch_map) => {
            let target_map = match target {
                serde_json::Value::Object(map) => map,
                _ => {
                    *target = serde_json::Value::Object(serde_json::Map::new());
                    match target {
                        serde_json::Value::Object(map) => map,
                        _ => unreachable!("target was just set to object"),
                    }
                }
            };
            for (key, value) in patch_map {
                if value.is_null() {
                    target_map.remove(key);
                } else {
                    rfc7396_merge_patch(
                        target_map.entry(key).or_insert(serde_json::Value::Null),
                        value,
                    );
                }
            }
        }
        _ => {
            *target = patch.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReplaceOptions, ResourceUpdater};
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipSpec, ShipStatus};
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, Time};

    fn ship() -> Ship {
        Ship {
            object_meta: Some(ObjectMeta {
                name: Some("demo".to_string()),
                namespace: Some("default".to_string()),
                uid: Some("uid-1".to_string()),
                resource_version: Some("rv-1".to_string()),
                generation: Some(3),
                creation_timestamp: Some(Time {
                    seconds: 1_700_000_000,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                image: "registry.example.com/vm:v1".to_string(),
                ship_class: "small".to_string(),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                conditions: vec![ShipCondition {
                    status: "Ready".to_string(),
                    message: "ok".to_string(),
                    timestamp: Some(Time::now()),
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn merge_patch_preserves_status_and_identity_fields() {
        let current = ship();
        let options = ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        };
        let patched: Ship = ResourceUpdater::new(&current, options)
            .apply_patch(
                serde_json::json!({
                    "metadata": {
                        "name": "changed",
                        "labels": {"app": "demo"},
                        "resourceVersion": "rv-2"
                    },
                    "spec": {
                        "image": "registry.example.com/vm:v2"
                    },
                    "status": {
                        "conditions": []
                    }
                })
                .as_object()
                .cloned()
                .expect("patch should be object"),
            )
            .expect("patch should merge");

        assert_eq!(patched.name(), Some("demo"));
        assert_eq!(patched.namespace(), Some("default"));
        assert_eq!(
            patched.spec.as_ref().map(|spec| spec.image.as_str()),
            Some("registry.example.com/vm:v2")
        );
        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.resource_version.as_deref()),
            Some("rv-1")
        );
        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.generation),
            Some(4)
        );
        assert_eq!(
            patched
                .status
                .as_ref()
                .map(|status| status.conditions.len()),
            Some(1)
        );
        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.labels.get("app"))
                .map(String::as_str),
            Some("demo")
        );
    }

    #[test]
    fn merge_patch_uses_client_resource_version_when_requested() {
        let current = ship();
        let options = ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
            update_generation: true,
        };
        let patched: Ship = ResourceUpdater::new(&current, options)
            .apply_patch(
                serde_json::json!({
                    "metadata": {
                        "resourceVersion": "rv-9"
                    }
                })
                .as_object()
                .cloned()
                .expect("patch should be object"),
            )
            .expect("patch should merge");

        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.resource_version.as_deref()),
            Some("rv-9")
        );
        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.generation),
            Some(3)
        );
    }

    #[test]
    fn merge_patch_bumps_generation_when_content_changes() {
        let current = ship();
        let options = ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        };
        let patched: Ship = ResourceUpdater::new(&current, options)
            .apply_patch(
                serde_json::json!({
                    "spec": {
                        "image": "registry.example.com/vm:v2"
                    }
                })
                .as_object()
                .cloned()
                .expect("patch should be object"),
            )
            .expect("patch should merge");

        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.generation),
            Some(4)
        );
    }

    #[test]
    fn status_updates_preserve_generation() {
        let current = ship();
        let updated: Ship = ResourceUpdater::new(&current, ReplaceOptions::default())
            .apply_status_update(serde_json::json!({
                "conditions": [
                    {
                        "status": "NotReady",
                        "message": "maintenance"
                    }
                ]
            }))
            .expect("status update should merge");

        assert_eq!(
            updated
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.generation),
            Some(3)
        );
    }

    #[test]
    fn rfc7396_merge_patch_replaces_arrays() {
        use super::rfc7396_merge_patch;
        use serde_json::json;

        let mut target = json!({
            "spec": {
                "components": [
                    { "name": "frontend", "replicas": 2 }
                ]
            }
        });
        let patch = json!({
            "spec": {
                "components": [
                    { "name": "frontend", "replicas": 3 },
                    { "name": "worker", "replicas": 1 }
                ]
            }
        });

        rfc7396_merge_patch(&mut target, &patch);

        let components = target["spec"]["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0]["replicas"], 3);
        assert_eq!(components[1]["name"], "worker");
    }

    #[test]
    fn rfc7396_merge_patch_removes_null_fields() {
        use super::rfc7396_merge_patch;
        use serde_json::json;

        let mut target = json!({ "a": 1, "b": 2 });
        rfc7396_merge_patch(&mut target, &json!({ "b": null }));

        assert_eq!(target["a"], 1);
        assert!(target.get("b").is_none() || target["b"].is_null());
    }

    #[test]
    fn rfc7396_merge_patch_merges_nested_objects() {
        use super::rfc7396_merge_patch;
        use serde_json::json;

        let mut target = json!({ "spec": { "image": "v1", "class": "small" } });
        rfc7396_merge_patch(&mut target, &json!({ "spec": { "image": "v2" } }));

        assert_eq!(target["spec"]["image"], "v2");
        assert_eq!(target["spec"]["class"], "small");
    }
}
