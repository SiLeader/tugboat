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
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta, StaticResource};

pub(crate) async fn create_cluster<T>(
    object: T,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<T>, StatusResponse>
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
) -> Result<ModifyResponse<T>, StatusResponse>
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
) -> Result<HttpResponse, StatusResponse>
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
        let resources = operator.store.list::<T>(namespace, None).await?;
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
) -> Result<ReadResponse<T>, StatusResponse>
where
    T: StaticSerializable + ObjectMetaResource + StaticResource + Serialize,
{
    let resource = operator.store.get::<T>(namespace.clone(), &name).await?;
    match resource {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )),
    }
}

pub(crate) async fn delete_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
) -> Result<ReadResponse<T>, StatusResponse>
where
    T: StaticSerializable + ObjectMetaResource + StaticResource + Serialize,
{
    let resource = operator.store.delete::<T>(namespace.clone(), &name).await?;
    match resource {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        )),
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
) -> Result<ModifyResponse<T>, StatusResponse>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq,
{
    let current = operator.store.get::<T>(namespace.clone(), &name).await?;
    let Some(current) = current else {
        return Err(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        ));
    };
    let current = current.apply_revision();
    let replaced = merge_replacement(&current, &replacement, options)?;

    let replaced = if current != replaced {
        operator.store.put(replaced).await?.apply_revision()
    } else {
        replaced
    };
    Ok(ModifyResponse::Updated(replaced))
}

pub(crate) async fn status_patch_resource<T>(
    operator: &ApiOperator,
    namespace: Option<String>,
    name: String,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<ModifyResponse<T>, StatusResponse>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq,
{
    if !patch.keys().all(|key| key == "status") {
        return Err(StatusResponse::bad_request(
            "PATCH /status must contain only status field.",
            None,
        ));
    }
    let patch = serde_json::Value::Object(patch);

    let current = operator.store.get::<T>(namespace.clone(), &name).await?;
    let Some(current) = current else {
        return Err(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        ));
    };
    let current = current.apply_revision();

    let mut patched = serde_json::to_value(&current)?;
    patched.merge(&patch);
    let patched = serde_json::from_value::<T>(patched)?;

    let patched = if current != patched {
        operator.store.put(patched).await?.apply_revision()
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
) -> Result<ModifyResponse<T>, StatusResponse>
where
    T: StaticSerializable
        + ObjectMetaResource
        + StaticResource
        + Serialize
        + DeserializeOwned
        + PartialEq,
{
    let current = operator.store.get::<T>(namespace.clone(), &name).await?;
    let Some(current) = current else {
        return Err(StatusResponse::not_found(
            format!("{} not found", T::kind()),
            Some(resource_identity(namespace.as_deref(), &name)),
        ));
    };
    let current = current.apply_revision();

    let replacement = serde_json::to_value(replacement)?;
    let status = replacement
        .as_object()
        .and_then(|obj| obj.get("status").cloned())
        .unwrap_or(serde_json::Value::Null);

    let mut replaced = to_object(serde_json::to_value(&current)?, "current resource")?;
    replaced.insert("status".to_string(), status);
    let replaced = serde_json::from_value::<T>(serde_json::Value::Object(replaced))?;

    let replaced = if current != replaced {
        operator.store.put(replaced).await?.apply_revision()
    } else {
        replaced
    };
    Ok(ModifyResponse::Updated(replaced))
}

fn merge_replacement<T>(
    current: &T,
    replacement: &T,
    options: ReplaceOptions,
) -> Result<T, StatusResponse>
where
    T: Serialize + DeserializeOwned,
{
    let mut merged = to_object(serde_json::to_value(current)?, "current resource")?;
    let replacement = to_object(serde_json::to_value(replacement)?, "replacement resource")?;

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
    if options.use_client_resource_version
        && let Some(client_rv) = replacement
            .get("metadata")
            .and_then(|meta| meta.get("resourceVersion"))
            .cloned()
    {
        let _ = metadata.insert("resourceVersion".to_string(), client_rv);
    }
    let _ = merged.insert("metadata".to_string(), serde_json::Value::Object(metadata));

    Ok(serde_json::from_value::<T>(serde_json::Value::Object(
        merged,
    ))?)
}

fn to_object(
    value: serde_json::Value,
    context: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, StatusResponse> {
    match value {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(StatusResponse::internal_error(
            format!("{context} is not a JSON object"),
            None,
        )),
    }
}

fn resource_identity(namespace: Option<&str>, name: &str) -> serde_json::Value {
    if let Some(namespace) = namespace {
        serde_json::json!({ "namespace": namespace, "name": name })
    } else {
        serde_json::json!({ "name": name })
    }
}
