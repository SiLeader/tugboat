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

use super::validation::validate_against_schema;
use super::watch::watch_custom;
use crate::crd_registry::{CrdEntry, CrdScope};
use crate::data::{ResourceList, StatusResponse};
use crate::endpoints::ListQuery;
use crate::endpoints::selector::{FieldSelector, Selector};
use crate::operator::ApiOperator;
use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse};
use tugboat_resource_store::ContentData;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::Namespace;
use tugboat_resources::manifests::meta::v1::{CustomResourceObject, ObjectMeta, Time, TypeMeta};
use uuid::Uuid;

pub(super) async fn create(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    mut value: serde_json::Value,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    if let Some(namespace) = namespace.as_deref() {
        ensure_namespace_exists(operator, namespace).await?;
    }

    enforce_type_meta(&mut value, &entry)?;
    let mut meta = extract_metadata(&value)?;
    match entry.scope {
        CrdScope::Namespaced => meta.namespace = namespace.clone(),
        CrdScope::Cluster => {
            if meta.namespace.is_some() {
                return Err(Box::new(StatusResponse::bad_request(
                    "metadata.namespace cannot be set",
                    None,
                )));
            }
        }
    }
    if meta.name.is_none() {
        let Some(generate_name) = meta.generate_name.clone() else {
            return Err(Box::new(StatusResponse::bad_request(
                "metadata.name or metadata.generateName is required",
                None,
            )));
        };
        meta.name = Some(operator.name_generator.generate(&generate_name).await);
    }
    meta = apply_new_metadata(meta);
    set_metadata(&mut value, &meta)?;
    normalize_status_for_write(&entry, &mut value)?;
    validate_against_schema(&entry, &value)?;

    let name = meta.name.clone().expect("metadata.name was populated");
    let envelope = envelope_from_value(&entry, &value, meta)?;
    let revision = operator
        .store
        .put_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            &envelope,
            Some(0),
        )
        .await?;
    let value = value_with_revision(value, revision)?;
    Ok(HttpResponse::Created().json(value))
}

pub(super) async fn list(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    query: ListQuery,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_list_scope(&entry, namespace.as_deref())?;

    if let Some(watch) = query.watch {
        return watch_custom(operator, entry, namespace, query, watch).await;
    }

    let field_selector = query.to_field_selector()?;
    let label_selector = query.to_label_selector()?;
    let items = operator
        .store
        .list_custom(&entry.group, &entry.plural, namespace.as_deref())
        .await?
        .into_iter()
        .filter_map(|data| custom_data_to_value(data).transpose())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|value| matches_selectors(value, &field_selector, &label_selector))
        .collect();

    Ok(ResourceList::from_raw_items(items).into())
}

pub(super) async fn read(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    name: String,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    Ok(HttpResponse::Ok().json(custom_data_to_value(current)?.expect("resource has raw JSON")))
}

pub(super) async fn replace(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    name: String,
    mut replacement: serde_json::Value,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let current_value = custom_data_to_value(current)?.expect("resource has raw JSON");
    validate_metadata_name(&replacement, &name)?;
    enforce_type_meta(&mut replacement, &entry)?;

    let current_meta = extract_metadata(&current_value)?;
    let mut replacement_meta = extract_metadata(&replacement)?;
    preserve_identity_metadata(&current_meta, &mut replacement_meta);
    replacement_meta.generation =
        if generation_tracked_fields(&current_value) != generation_tracked_fields(&replacement) {
            next_generation(current_meta.generation)
        } else {
            current_meta.generation
        };
    if entry.version.status_subresource {
        preserve_status(&mut replacement, &current_value)?;
    }
    set_metadata(&mut replacement, &replacement_meta)?;
    normalize_status_for_write(&entry, &mut replacement)?;
    validate_against_schema(&entry, &replacement)?;

    let envelope = envelope_from_value(&entry, &replacement, replacement_meta)?;
    let revision = operator
        .store
        .put_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            &envelope,
            resource_version_as_revision(&current_value),
        )
        .await?;
    inject_resource_version(&mut replacement, revision)?;
    Ok(HttpResponse::Ok().json(replacement))
}

pub(super) async fn patch(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    name: String,
    patch: serde_json::Value,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let patch = patch_object(patch)?;
    validate_patch_name(&patch, &name)?;
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let mut current_value = custom_data_to_value(current)?.expect("resource has raw JSON");
    let old_value = current_value.clone();
    rfc7396_merge_patch(&mut current_value, &serde_json::Value::Object(patch));
    enforce_type_meta(&mut current_value, &entry)?;
    preserve_identity_and_maybe_status(&entry, &mut current_value, &old_value)?;
    normalize_status_for_write(&entry, &mut current_value)?;
    validate_against_schema(&entry, &current_value)?;

    let meta = extract_metadata(&current_value)?;
    let envelope = envelope_from_value(&entry, &current_value, meta)?;
    let revision = operator
        .store
        .put_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            &envelope,
            resource_version_as_revision(&old_value),
        )
        .await?;
    inject_resource_version(&mut current_value, revision)?;
    Ok(HttpResponse::Ok().json(current_value))
}

pub(super) async fn delete(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    name: String,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let mut current_value = custom_data_to_value(current)?.expect("resource has raw JSON");

    if has_finalizers(&current_value) {
        set_deletion_timestamp(&mut current_value)?;
        let meta = extract_metadata(&current_value)?;
        let envelope = envelope_from_value(&entry, &current_value, meta)?;
        let revision = operator
            .store
            .put_custom(
                &entry.group,
                &entry.plural,
                namespace.as_deref(),
                &name,
                &envelope,
                resource_version_as_revision(&current_value),
            )
            .await?;
        inject_resource_version(&mut current_value, revision)?;
        return Ok(HttpResponse::Ok().json(current_value));
    }

    operator
        .store
        .delete_custom(&entry.group, &entry.plural, namespace.as_deref(), &name)
        .await?;
    Ok(HttpResponse::Ok().json(current_value))
}

pub(super) async fn replace_status(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    name: String,
    replacement: serde_json::Value,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let entry = lookup_status(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let mut current_value = custom_data_to_value(current)?.expect("resource has raw JSON");
    let old_value = current_value.clone();
    let status = replacement
        .as_object()
        .and_then(|obj| obj.get("status").cloned())
        .unwrap_or(serde_json::Value::Null);
    set_status(&mut current_value, status)?;
    preserve_identity_metadata_from_value(&mut current_value, &old_value)?;
    validate_against_schema(&entry, &current_value)?;
    let meta = extract_metadata(&current_value)?;
    let envelope = envelope_from_value(&entry, &current_value, meta)?;
    let revision = operator
        .store
        .put_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            &envelope,
            resource_version_as_revision(&old_value),
        )
        .await?;
    inject_resource_version(&mut current_value, revision)?;
    Ok(HttpResponse::Ok().json(current_value))
}

pub(super) async fn patch_status(
    operator: &ApiOperator,
    group: String,
    version: String,
    namespace: Option<String>,
    plural: String,
    name: String,
    patch: serde_json::Value,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let patch = patch_object(patch)?;
    let entry = lookup_status(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let mut current_value = custom_data_to_value(current)?.expect("resource has raw JSON");
    let old_value = current_value.clone();
    let status_patch = patch.get("status").cloned().ok_or_else(|| {
        Box::new(StatusResponse::bad_request(
            "status field is required",
            None,
        ))
    })?;
    rfc7396_merge_patch(
        &mut current_value,
        &serde_json::json!({ "status": status_patch }),
    );
    preserve_identity_metadata_from_value(&mut current_value, &old_value)?;
    validate_against_schema(&entry, &current_value)?;
    let meta = extract_metadata(&current_value)?;
    let envelope = envelope_from_value(&entry, &current_value, meta)?;
    let revision = operator
        .store
        .put_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            &envelope,
            resource_version_as_revision(&old_value),
        )
        .await?;
    inject_resource_version(&mut current_value, revision)?;
    Ok(HttpResponse::Ok().json(current_value))
}

fn lookup(
    operator: &ApiOperator,
    group: &str,
    version: &str,
    plural: &str,
) -> Result<CrdEntry, Box<StatusResponse>> {
    operator
        .crd_registry
        .lookup(group, version, plural)
        .ok_or_else(|| {
            Box::new(StatusResponse::not_found(
                format!("Custom resource {group}/{version}/{plural} not found"),
                Some(serde_json::json!({
                    "group": group,
                    "version": version,
                    "plural": plural,
                })),
            ))
        })
}

fn lookup_status(
    operator: &ApiOperator,
    group: &str,
    version: &str,
    plural: &str,
) -> Result<CrdEntry, Box<StatusResponse>> {
    let entry = lookup(operator, group, version, plural)?;
    if entry.version.status_subresource {
        Ok(entry)
    } else {
        Err(Box::new(StatusResponse::not_found(
            format!("Status subresource for {group}/{version}/{plural} not found"),
            None,
        )))
    }
}

fn ensure_write_scope(
    entry: &CrdEntry,
    namespace: Option<&str>,
) -> Result<(), Box<StatusResponse>> {
    match (entry.scope, namespace) {
        (CrdScope::Namespaced, None) => Err(Box::new(StatusResponse::bad_request(
            "namespace required for namespaced custom resource",
            None,
        ))),
        (CrdScope::Cluster, Some(_)) => Err(Box::new(StatusResponse::bad_request(
            "namespace not allowed for cluster-scoped custom resource",
            None,
        ))),
        _ => Ok(()),
    }
}

fn ensure_list_scope(entry: &CrdEntry, namespace: Option<&str>) -> Result<(), Box<StatusResponse>> {
    if entry.scope == CrdScope::Cluster && namespace.is_some() {
        return Err(Box::new(StatusResponse::bad_request(
            "namespace not allowed for cluster-scoped custom resource",
            None,
        )));
    }
    Ok(())
}

async fn ensure_namespace_exists(
    operator: &ApiOperator,
    namespace: &str,
) -> Result<(), Box<StatusResponse>> {
    if operator
        .store
        .get::<Namespace>(None, namespace)
        .await?
        .is_some()
    {
        Ok(())
    } else {
        Err(Box::new(StatusResponse::not_found(
            format!("namespaces \"{namespace}\" not found"),
            Some(serde_json::json!({ "name": namespace })),
        )))
    }
}

async fn get_existing(
    operator: &ApiOperator,
    entry: &CrdEntry,
    namespace: Option<&str>,
    name: &str,
) -> Result<ContentData<CustomResourceObject>, Box<StatusResponse>> {
    operator
        .store
        .get_custom(&entry.group, &entry.plural, namespace, name)
        .await?
        .ok_or_else(|| {
            Box::new(StatusResponse::not_found(
                format!("{} \"{name}\" not found", entry.kind),
                Some(resource_identity(namespace, name)),
            ))
        })
}

pub(super) fn custom_data_to_value(
    data: ContentData<CustomResourceObject>,
) -> Result<Option<serde_json::Value>, Box<StatusResponse>> {
    let mut envelope = data.data;
    if let Some(meta) = envelope.object_meta_mut().as_mut() {
        meta.resource_version = Some(data.revision.to_string());
    }
    let mut value = value_from_envelope(&envelope)?;
    inject_resource_version(&mut value, data.revision)?;
    Ok(Some(value))
}

fn value_from_envelope(
    envelope: &CustomResourceObject,
) -> Result<serde_json::Value, Box<StatusResponse>> {
    let value = serde_json::from_slice(&envelope.raw_json)?;
    Ok(value)
}

fn envelope_from_value(
    entry: &CrdEntry,
    value: &serde_json::Value,
    object_meta: ObjectMeta,
) -> Result<CustomResourceObject, Box<StatusResponse>> {
    Ok(CustomResourceObject {
        type_meta: Some(TypeMeta {
            api_version: Some(format!("{}/{}", entry.group, entry.version.name)),
            kind: Some(entry.kind.clone()),
        }),
        object_meta: Some(object_meta),
        raw_json: serde_json::to_vec(value)?,
    })
}

fn enforce_type_meta(
    value: &mut serde_json::Value,
    entry: &CrdEntry,
) -> Result<(), Box<StatusResponse>> {
    let expected_api_version = format!("{}/{}", entry.group, entry.version.name);
    let obj = value.as_object_mut().ok_or_else(|| {
        Box::new(StatusResponse::bad_request(
            "custom resource body must be a JSON object",
            None,
        ))
    })?;
    enforce_string_field(obj, "apiVersion", &expected_api_version)?;
    enforce_string_field(obj, "kind", &entry.kind)?;
    Ok(())
}

fn enforce_string_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    expected: &str,
) -> Result<(), Box<StatusResponse>> {
    match obj.get(field).and_then(|v| v.as_str()) {
        Some(actual) if actual != expected => Err(Box::new(StatusResponse::bad_request(
            format!("{field} must be \"{expected}\", got \"{actual}\""),
            Some(serde_json::json!({ "field": field, "expected": expected, "actual": actual })),
        ))),
        Some(_) => Ok(()),
        None => {
            obj.insert(
                field.to_string(),
                serde_json::Value::String(expected.to_string()),
            );
            Ok(())
        }
    }
}

fn extract_metadata(value: &serde_json::Value) -> Result<ObjectMeta, Box<StatusResponse>> {
    let metadata = value
        .get("metadata")
        .cloned()
        .ok_or_else(|| Box::new(StatusResponse::bad_request("metadata is required", None)))?;
    serde_json::from_value(metadata).map_err(|err| {
        Box::new(StatusResponse::bad_request(
            "metadata is invalid",
            Some(serde_json::json!({"error": err.to_string()})),
        ))
    })
}

fn set_metadata(
    value: &mut serde_json::Value,
    meta: &ObjectMeta,
) -> Result<(), Box<StatusResponse>> {
    let obj = value.as_object_mut().ok_or_else(|| {
        Box::new(StatusResponse::bad_request(
            "custom resource body must be a JSON object",
            None,
        ))
    })?;
    obj.insert("metadata".to_string(), serde_json::to_value(meta)?);
    Ok(())
}

fn apply_new_metadata(mut meta: ObjectMeta) -> ObjectMeta {
    meta.uid = Some(Uuid::new_v4().to_string());
    meta.creation_timestamp = Some(Time::now());
    meta.generation = Some(1);
    meta.resource_version = None;
    meta
}

fn preserve_identity_metadata(current: &ObjectMeta, next: &mut ObjectMeta) {
    next.name = current.name.clone();
    next.namespace = current.namespace.clone();
    next.uid = current.uid.clone();
    next.creation_timestamp = current.creation_timestamp;
    next.deletion_timestamp = current.deletion_timestamp;
    next.resource_version = current.resource_version.clone();
}

fn preserve_identity_metadata_from_value(
    next_value: &mut serde_json::Value,
    current_value: &serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let current_meta = extract_metadata(current_value)?;
    let mut next_meta = extract_metadata(next_value)?;
    preserve_identity_metadata(&current_meta, &mut next_meta);
    next_meta.generation = current_meta.generation;
    set_metadata(next_value, &next_meta)
}

fn preserve_identity_and_maybe_status(
    entry: &CrdEntry,
    next_value: &mut serde_json::Value,
    current_value: &serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let current_meta = extract_metadata(current_value)?;
    let mut next_meta = extract_metadata(next_value)?;
    preserve_identity_metadata(&current_meta, &mut next_meta);
    next_meta.generation =
        if generation_tracked_fields(current_value) != generation_tracked_fields(next_value) {
            next_generation(current_meta.generation)
        } else {
            current_meta.generation
        };
    set_metadata(next_value, &next_meta)?;
    if entry.version.status_subresource {
        preserve_status(next_value, current_value)?;
    }
    Ok(())
}

fn preserve_status(
    next_value: &mut serde_json::Value,
    current_value: &serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let current_status = current_value.get("status").cloned();
    let obj = next_value.as_object_mut().ok_or_else(|| {
        Box::new(StatusResponse::bad_request(
            "custom resource body must be a JSON object",
            None,
        ))
    })?;
    if let Some(status) = current_status {
        obj.insert("status".to_string(), status);
    } else {
        obj.remove("status");
    }
    Ok(())
}

fn set_status(
    value: &mut serde_json::Value,
    status: serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let obj = value.as_object_mut().ok_or_else(|| {
        Box::new(StatusResponse::bad_request(
            "custom resource body must be a JSON object",
            None,
        ))
    })?;
    obj.insert("status".to_string(), status);
    Ok(())
}

fn normalize_status_for_write(
    entry: &CrdEntry,
    value: &mut serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    if !entry.version.status_subresource {
        return Ok(());
    }
    let obj = value.as_object_mut().ok_or_else(|| {
        Box::new(StatusResponse::bad_request(
            "custom resource body must be a JSON object",
            None,
        ))
    })?;
    obj.entry("status".to_string())
        .or_insert_with(|| serde_json::json!({}));
    Ok(())
}

fn validate_metadata_name(
    value: &serde_json::Value,
    expected_name: &str,
) -> Result<(), Box<StatusResponse>> {
    let actual = value
        .get("metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(|name| name.as_str());
    if let Some(actual) = actual
        && actual != expected_name
    {
        return Err(Box::new(StatusResponse::bad_request(
            format!(
                "metadata.name must match resource name in URL: expected \"{expected_name}\", got \"{actual}\""
            ),
            Some(serde_json::json!({
                "name": expected_name,
                "providedName": actual,
            })),
        )));
    }
    Ok(())
}

fn validate_patch_name(
    patch: &serde_json::Map<String, serde_json::Value>,
    expected_name: &str,
) -> Result<(), Box<StatusResponse>> {
    let actual = patch
        .get("metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(|name| name.as_str());
    if let Some(actual) = actual
        && actual != expected_name
    {
        return Err(Box::new(StatusResponse::bad_request(
            format!(
                "metadata.name must match resource name in URL: expected \"{expected_name}\", got \"{actual}\""
            ),
            Some(serde_json::json!({
                "name": expected_name,
                "providedName": actual,
            })),
        )));
    }
    Ok(())
}

fn value_with_revision(
    mut value: serde_json::Value,
    revision: i64,
) -> Result<serde_json::Value, Box<StatusResponse>> {
    inject_resource_version(&mut value, revision)?;
    Ok(value)
}

fn inject_resource_version(
    value: &mut serde_json::Value,
    revision: i64,
) -> Result<(), Box<StatusResponse>> {
    let mut meta = extract_metadata(value)?;
    meta.resource_version = Some(revision.to_string());
    set_metadata(value, &meta)
}

fn resource_version_as_revision(value: &serde_json::Value) -> Option<i64> {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get("resourceVersion"))
        .and_then(|rv| rv.as_str())
        .and_then(|rv| rv.parse::<i64>().ok())
}

fn patch_object(
    patch: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, Box<StatusResponse>> {
    match patch {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(Box::new(StatusResponse::bad_request(
            "JSON merge patch body must be an object",
            None,
        ))),
    }
}

pub(super) fn ensure_merge_patch_content_type(
    req: &HttpRequest,
) -> Result<(), Box<StatusResponse>> {
    let Some(content_type) = req.headers().get(header::CONTENT_TYPE) else {
        return Err(Box::new(unsupported_media_type()));
    };
    let Ok(content_type) = content_type.to_str() else {
        return Err(Box::new(unsupported_media_type()));
    };
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if media_type == "application/merge-patch+json" {
        Ok(())
    } else {
        Err(Box::new(unsupported_media_type()))
    }
}

fn unsupported_media_type() -> StatusResponse {
    StatusResponse::with_all(
        "Unsupported patch content type".to_string(),
        "UnsupportedMediaType".to_string(),
        415,
        Some(serde_json::json!({
            "supportedMediaTypes": ["application/merge-patch+json"],
        })),
    )
}

pub(super) fn matches_selectors(
    value: &serde_json::Value,
    field_selector: &Option<Vec<Selector>>,
    label_selector: &Option<Vec<Selector>>,
) -> bool {
    if let Some(field_selector) = field_selector {
        let field_selector = field_selector
            .iter()
            .cloned()
            .map(FieldSelector::from)
            .collect::<Vec<_>>();
        if !field_selector
            .iter()
            .all(|selector| selector.is_match(value))
        {
            return false;
        }
    }
    if let Some(label_selector) = label_selector {
        let Ok(meta) = extract_metadata(value) else {
            return false;
        };
        if !label_selector
            .iter()
            .all(|selector| selector.is_label_match(&meta))
        {
            return false;
        }
    }
    true
}

fn generation_tracked_fields(
    value: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    value
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(key, _)| {
                    !matches!(key.as_str(), "metadata" | "apiVersion" | "kind" | "status")
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn next_generation(current: Option<i64>) -> Option<i64> {
    Some(current.unwrap_or(0).max(0).saturating_add(1))
}

fn has_finalizers(value: &serde_json::Value) -> bool {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get("finalizers"))
        .and_then(|finalizers| finalizers.as_array())
        .is_some_and(|finalizers| !finalizers.is_empty())
}

fn set_deletion_timestamp(value: &mut serde_json::Value) -> Result<(), Box<StatusResponse>> {
    let mut meta = extract_metadata(value)?;
    if meta.deletion_timestamp.is_none() {
        meta.deletion_timestamp = Some(Time::now());
        set_metadata(value, &meta)?;
    }
    Ok(())
}

fn resource_identity(namespace: Option<&str>, name: &str) -> serde_json::Value {
    if let Some(namespace) = namespace {
        serde_json::json!({ "namespace": namespace, "name": name })
    } else {
        serde_json::json!({ "name": name })
    }
}

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
    use super::{
        ensure_list_scope, ensure_write_scope, generation_tracked_fields, next_generation,
    };
    use crate::crd_registry::{CrdEntry, CrdScope, CrdVersionInfo};
    use actix_web::ResponseError;
    use actix_web::http::StatusCode;
    use serde_json::json;

    #[test]
    fn write_scope_rejects_namespaced_resource_without_namespace() {
        let err = ensure_write_scope(&entry(CrdScope::Namespaced), None)
            .expect_err("namespaced resource should require namespace");

        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn write_scope_rejects_cluster_resource_with_namespace() {
        let err = ensure_write_scope(&entry(CrdScope::Cluster), Some("default"))
            .expect_err("cluster resource should reject namespace");

        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn list_scope_allows_cluster_collection_without_namespace() {
        assert!(ensure_list_scope(&entry(CrdScope::Cluster), None).is_ok());
    }

    #[test]
    fn generation_ignores_metadata_type_meta_and_status() {
        let tracked = generation_tracked_fields(&json!({
            "apiVersion": "example.com/v1",
            "kind": "Widget",
            "metadata": {"name": "demo"},
            "spec": {"size": 1},
            "status": {"phase": "Ready"}
        }));

        assert_eq!(
            tracked,
            serde_json::Map::from_iter([("spec".to_string(), json!({"size": 1}))])
        );
        assert_eq!(next_generation(Some(1)), Some(2));
    }

    fn entry(scope: CrdScope) -> CrdEntry {
        CrdEntry {
            group: "example.com".to_string(),
            plural: "widgets".to_string(),
            singular: "widget".to_string(),
            kind: "Widget".to_string(),
            list_kind: "WidgetList".to_string(),
            scope,
            version: CrdVersionInfo {
                name: "v1".to_string(),
                served: true,
                storage: true,
                schema_json: None,
                compiled_schema: None,
                status_subresource: false,
            },
        }
    }
}
