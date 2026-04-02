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
use json_value_merge::Merge;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tugboat_resource_store::serializer::StaticSerializable;
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta, StaticResource};

pub(crate) async fn create_cluster<T>(
    object: T,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: Resource + StaticSerializable + ObjectMetaResource + SetTypeMeta + Clone,
{
    let object_meta = crate::extract_object_meta!(object);
    crate::check_namespace_absent!(object_meta);

    crate::create_object!(operator, object_meta, object, T::type_meta())
}

pub(crate) async fn create_namespaced<T>(
    object: T,
    namespace: String,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<T>, Box<StatusResponse>>
where
    T: Resource + StaticSerializable + ObjectMetaResource + SetTypeMeta + Clone,
{
    let object_meta = crate::extract_object_meta!(object);
    let object_meta = operator.apply_namespace(object_meta, namespace);

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

#[derive(Clone, Copy)]
pub(crate) struct ReplaceOptions {
    pub(crate) preserve_status: bool,
    pub(crate) use_client_resource_version: bool,
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
        + PartialEq,
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
    let replaced = merge_replacement(&current, &replacement, options)?;

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
        + PartialEq,
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
    let patched = merge_patch(&current, patch, options)?;

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
        + PartialEq,
{
    if !patch.keys().all(|key| key == "status") {
        return Err(Box::new(StatusResponse::bad_request(
            "PATCH /status must contain only status field.",
            None,
        )));
    }
    let patch = serde_json::Value::Object(patch);

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

    let mut patched = serde_json::to_value(&current).map_err(|e| Box::new(e.into()))?;
    patched.merge(&patch);
    let patched = serde_json::from_value::<T>(patched).map_err(|e| Box::new(e.into()))?;

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
        + PartialEq,
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

    let replacement = serde_json::to_value(replacement).map_err(|e| Box::new(e.into()))?;
    let status = replacement
        .as_object()
        .and_then(|obj| obj.get("status").cloned())
        .unwrap_or(serde_json::Value::Null);

    let mut replaced = to_object(
        serde_json::to_value(&current).map_err(|e| Box::new(e.into()))?,
        "current resource",
    )?;
    replaced.insert("status".to_string(), status);
    let replaced = serde_json::from_value::<T>(serde_json::Value::Object(replaced))
        .map_err(|e| Box::new(e.into()))?;

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

fn merge_replacement<T>(
    current: &T,
    replacement: &T,
    options: ReplaceOptions,
) -> Result<T, Box<StatusResponse>>
where
    T: Serialize + DeserializeOwned,
{
    let mut merged = to_object(
        serde_json::to_value(current).map_err(|e| Box::new(e.into()))?,
        "current resource",
    )?;
    let replacement = to_object(
        serde_json::to_value(replacement).map_err(|e| Box::new(e.into()))?,
        "replacement resource",
    )?;

    for (key, value) in &replacement {
        if key == "metadata" || key == "apiVersion" || key == "kind" {
            continue;
        }
        if options.preserve_status && key == "status" {
            continue;
        }
        let _ = merged.insert(key.clone(), value.clone());
    }

    let mut metadata = merged
        .remove("metadata")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if options.use_client_resource_version {
        if let Some(client_rv) = replacement
            .get("metadata")
            .and_then(|meta| meta.get("resourceVersion"))
            .cloned()
        {
            let _ = metadata.insert("resourceVersion".to_string(), client_rv);
        } else {
            // If client didn't provide resourceVersion, remove it from metadata
            // so that it becomes None (unconditional update).
            let _ = metadata.remove("resourceVersion");
        }
    }
    let _ = merged.insert("metadata".to_string(), serde_json::Value::Object(metadata));

    serde_json::from_value::<T>(serde_json::Value::Object(merged)).map_err(|e| Box::new(e.into()))
}

fn merge_patch<T>(
    current: &T,
    patch: serde_json::Map<String, serde_json::Value>,
    options: ReplaceOptions,
) -> Result<T, Box<StatusResponse>>
where
    T: Serialize + DeserializeOwned,
{
    let current_object = to_object(
        serde_json::to_value(current).map_err(|e| Box::new(e.into()))?,
        "current resource",
    )?;
    let patch_value = serde_json::Value::Object(patch.clone());

    let mut merged = serde_json::Value::Object(current_object.clone());
    merged.merge(&patch_value);
    let mut merged = to_object(merged, "patched resource")?;

    if let Some(value) = current_object.get("apiVersion").cloned() {
        let _ = merged.insert("apiVersion".to_string(), value);
    }
    if let Some(value) = current_object.get("kind").cloned() {
        let _ = merged.insert("kind".to_string(), value);
    }

    if options.preserve_status {
        match current_object.get("status").cloned() {
            Some(status) => {
                let _ = merged.insert("status".to_string(), status);
            }
            None => {
                let _ = merged.remove("status");
            }
        }
    }

    let current_metadata = current_object
        .get("metadata")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let patch_metadata = patch
        .get("metadata")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut merged_metadata = merged
        .remove("metadata")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    for key in [
        "name",
        "namespace",
        "uid",
        "generation",
        "creationTimestamp",
        "deletionTimestamp",
    ] {
        if let Some(value) = current_metadata.get(key).cloned() {
            let _ = merged_metadata.insert(key.to_string(), value);
        }
    }

    if options.use_client_resource_version {
        if let Some(client_rv) = patch_metadata.get("resourceVersion").cloned() {
            let _ = merged_metadata.insert("resourceVersion".to_string(), client_rv);
        } else {
            let _ = merged_metadata.remove("resourceVersion");
        }
    } else if let Some(value) = current_metadata.get("resourceVersion").cloned() {
        let _ = merged_metadata.insert("resourceVersion".to_string(), value);
    }

    let _ = merged.insert(
        "metadata".to_string(),
        serde_json::Value::Object(merged_metadata),
    );

    serde_json::from_value::<T>(serde_json::Value::Object(merged)).map_err(|e| Box::new(e.into()))
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

#[cfg(test)]
mod tests {
    use super::{ReplaceOptions, merge_patch};
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
        let patched = merge_patch(
            &current,
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
            ReplaceOptions {
                preserve_status: true,
                use_client_resource_version: false,
            },
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
        let patched = merge_patch(
            &current,
            serde_json::json!({
                "metadata": {
                    "resourceVersion": "rv-9"
                }
            })
            .as_object()
            .cloned()
            .expect("patch should be object"),
            ReplaceOptions {
                preserve_status: false,
                use_client_resource_version: true,
            },
        )
        .expect("patch should merge");

        assert_eq!(
            patched
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.resource_version.as_deref()),
            Some("rv-9")
        );
    }
}
