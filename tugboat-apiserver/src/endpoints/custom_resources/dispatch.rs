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

use super::envelope::{
    custom_data_to_value as envelope_to_value, enforce_type_meta, envelope_from_value,
};
use super::merge::{patch_object, rfc7396_merge_patch};
use super::metadata::{
    apply_new_metadata, extract_metadata, has_finalizers, inject_resource_version,
    normalize_status_for_write, parse_client_resource_version, preserve_identity_and_maybe_status,
    preserve_identity_metadata, preserve_identity_metadata_from_value, preserve_status,
    resource_version_as_revision, set_deletion_timestamp, set_metadata, set_status,
    validate_create_name, validate_metadata_name, validate_patch_name, value_with_revision,
};
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
use tugboat_resources::manifests::core::v1::Namespace;
use tugboat_resources::manifests::meta::v1::CustomResourceObject;

pub(super) use super::envelope::custom_data_to_value;

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
        validate_create_name(&meta)?;
        meta.name = Some(operator.name_generator.generate(&generate_name).await);
    } else {
        validate_create_name(&meta)?;
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
        .filter_map(|data| envelope_to_value(data).transpose())
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
    Ok(HttpResponse::Ok().json(envelope_to_value(current)?.expect("resource has raw JSON")))
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
    let current_value = envelope_to_value(current)?.expect("resource has raw JSON");
    validate_metadata_name(&replacement, &name)?;
    enforce_type_meta(&mut replacement, &entry)?;

    let current_meta = extract_metadata(&current_value)?;
    let mut replacement_meta = extract_metadata(&replacement)?;
    // Capture the client-supplied resourceVersion before identity preservation
    // overwrites it. When present, it drives optimistic concurrency control;
    // when absent, we fall back to the value just read from the store.
    let client_resource_version = replacement_meta.resource_version.clone();
    preserve_identity_metadata(&current_meta, &mut replacement_meta);
    replacement_meta.generation = compute_generation(&current_value, &replacement, &current_meta);
    if entry.version.status_subresource {
        preserve_status(&mut replacement, &current_value)?;
    }
    set_metadata(&mut replacement, &replacement_meta)?;
    normalize_status_for_write(&entry, &mut replacement)?;
    validate_against_schema(&entry, &replacement)?;

    let expected_revision = parse_client_resource_version(client_resource_version.as_deref())?
        .or_else(|| resource_version_as_revision(&current_value));

    let envelope = envelope_from_value(&entry, &replacement, replacement_meta)?;
    let revision = operator
        .store
        .put_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            &envelope,
            expected_revision,
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
    let mut patch = patch_object(patch)?;
    // apiVersion and kind cannot be changed by a patch. Drop them before
    // merging so a stale client-supplied value does not race against
    // enforce_type_meta below; the type-meta is re-applied from the CRD
    // registry entry, which is the only source of truth.
    patch.remove("apiVersion");
    patch.remove("kind");
    validate_patch_name(&patch, &name)?;
    let entry = lookup(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let mut current_value = envelope_to_value(current)?.expect("resource has raw JSON");
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
    let mut current_value = envelope_to_value(current)?.expect("resource has raw JSON");

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
        .delete_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            &name,
            resource_version_as_revision(&current_value),
        )
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
    let mut current_value = envelope_to_value(current)?.expect("resource has raw JSON");
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
    // The /status subresource only accepts changes to the status field.
    // Rejecting other keys explicitly matches the built-in status_patch
    // handler and prevents silent loss of client intent.
    if !patch.keys().all(|key| key == "status") {
        return Err(Box::new(StatusResponse::bad_request(
            "PATCH /status must contain only the status field",
            None,
        )));
    }
    let entry = lookup_status(operator, &group, &version, &plural)?;
    ensure_write_scope(&entry, namespace.as_deref())?;
    let current = get_existing(operator, &entry, namespace.as_deref(), &name).await?;
    let mut current_value = envelope_to_value(current)?.expect("resource has raw JSON");
    let old_value = current_value.clone();
    if let Some(status_patch) = patch.get("status").cloned() {
        rfc7396_merge_patch(
            &mut current_value,
            &serde_json::json!({ "status": status_patch }),
        );
    }
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

fn compute_generation(
    current_value: &serde_json::Value,
    next_value: &serde_json::Value,
    current_meta: &tugboat_resources::manifests::meta::v1::ObjectMeta,
) -> Option<i64> {
    use super::metadata::{generation_tracked_fields, next_generation};
    if generation_tracked_fields(current_value) != generation_tracked_fields(next_value) {
        next_generation(current_meta.generation)
    } else {
        current_meta.generation
    }
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

fn resource_identity(namespace: Option<&str>, name: &str) -> serde_json::Value {
    if let Some(namespace) = namespace {
        serde_json::json!({ "namespace": namespace, "name": name })
    } else {
        serde_json::json!({ "name": name })
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_list_scope, ensure_write_scope};
    use crate::crd_registry::{CrdEntry, CrdScope, CrdVersionInfo};
    use actix_web::ResponseError;
    use actix_web::http::StatusCode;

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
