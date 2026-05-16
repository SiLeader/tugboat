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

use crate::crd_registry::CrdEntry;
use crate::data::StatusResponse;
use tugboat_resources::manifests::meta::v1::{ObjectMeta, Time};
use tugboat_resources::validators::NameValidator;
use uuid::Uuid;

pub(super) fn extract_metadata(
    value: &serde_json::Value,
) -> Result<ObjectMeta, Box<StatusResponse>> {
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

pub(super) fn set_metadata(
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

pub(super) fn apply_new_metadata(mut meta: ObjectMeta) -> ObjectMeta {
    meta.uid = Some(Uuid::new_v4().to_string());
    meta.creation_timestamp = Some(Time::now());
    meta.generation = Some(1);
    meta.resource_version = None;
    meta
}

pub(super) fn validate_create_name(meta: &ObjectMeta) -> Result<(), Box<StatusResponse>> {
    if let Some(name) = meta.name.as_deref() {
        return if NameValidator::is_valid_name(name) {
            Ok(())
        } else {
            Err(Box::new(StatusResponse::bad_request(
                "metadata.name is invalid",
                Some(serde_json::json!({ "name": name })),
            )))
        };
    }

    if let Some(generate_name) = meta.generate_name.as_deref() {
        return if NameValidator::is_valid_generate_name(generate_name) {
            Ok(())
        } else {
            Err(Box::new(StatusResponse::bad_request(
                "metadata.generateName is invalid",
                Some(serde_json::json!({ "generateName": generate_name })),
            )))
        };
    }

    Ok(())
}

pub(super) fn preserve_identity_metadata(current: &ObjectMeta, next: &mut ObjectMeta) {
    next.name = current.name.clone();
    next.namespace = current.namespace.clone();
    next.uid = current.uid.clone();
    next.creation_timestamp = current.creation_timestamp;
    next.deletion_timestamp = current.deletion_timestamp;
    next.resource_version = current.resource_version.clone();
}

pub(super) fn preserve_identity_metadata_from_value(
    next_value: &mut serde_json::Value,
    current_value: &serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let current_meta = extract_metadata(current_value)?;
    let mut next_meta = extract_metadata(next_value)?;
    preserve_identity_metadata(&current_meta, &mut next_meta);
    next_meta.generation = current_meta.generation;
    set_metadata(next_value, &next_meta)
}

pub(super) fn preserve_identity_and_maybe_status(
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

pub(super) fn preserve_status(
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

pub(super) fn set_status(
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

pub(super) fn normalize_status_for_write(
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

pub(super) fn validate_metadata_name(
    value: &serde_json::Value,
    expected_name: &str,
) -> Result<(), Box<StatusResponse>> {
    let actual = value
        .get("metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(|name| name.as_str());
    name_mismatch(actual, expected_name)
}

pub(super) fn validate_patch_name(
    patch: &serde_json::Map<String, serde_json::Value>,
    expected_name: &str,
) -> Result<(), Box<StatusResponse>> {
    let actual = patch
        .get("metadata")
        .and_then(|metadata| metadata.get("name"))
        .and_then(|name| name.as_str());
    name_mismatch(actual, expected_name)
}

fn name_mismatch(actual: Option<&str>, expected_name: &str) -> Result<(), Box<StatusResponse>> {
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

pub(super) fn inject_resource_version(
    value: &mut serde_json::Value,
    revision: i64,
) -> Result<(), Box<StatusResponse>> {
    let mut meta = extract_metadata(value)?;
    meta.resource_version = Some(revision.to_string());
    set_metadata(value, &meta)
}

pub(super) fn value_with_revision(
    mut value: serde_json::Value,
    revision: i64,
) -> Result<serde_json::Value, Box<StatusResponse>> {
    inject_resource_version(&mut value, revision)?;
    Ok(value)
}

pub(super) fn resource_version_as_revision(value: &serde_json::Value) -> Option<i64> {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get("resourceVersion"))
        .and_then(|rv| rv.as_str())
        .and_then(|rv| rv.parse::<i64>().ok())
}

pub(super) fn generation_tracked_fields(
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

pub(super) fn next_generation(current: Option<i64>) -> Option<i64> {
    Some(current.unwrap_or(0).max(0).saturating_add(1))
}

pub(super) fn has_finalizers(value: &serde_json::Value) -> bool {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get("finalizers"))
        .and_then(|finalizers| finalizers.as_array())
        .is_some_and(|finalizers| !finalizers.is_empty())
}

pub(super) fn set_deletion_timestamp(
    value: &mut serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let mut meta = extract_metadata(value)?;
    if meta.deletion_timestamp.is_none() {
        meta.deletion_timestamp = Some(Time::now());
        set_metadata(value, &meta)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{generation_tracked_fields, next_generation, validate_create_name};
    use serde_json::json;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

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

    #[test]
    fn validate_create_name_rejects_path_separators() {
        let meta = ObjectMeta {
            name: Some("a/b".to_string()),
            ..Default::default()
        };

        assert!(validate_create_name(&meta).is_err());
    }

    #[test]
    fn validate_create_name_rejects_empty_name() {
        let meta = ObjectMeta {
            name: Some(String::new()),
            ..Default::default()
        };

        assert!(validate_create_name(&meta).is_err());
    }

    #[test]
    fn validate_create_name_rejects_invalid_generate_name() {
        let meta = ObjectMeta {
            generate_name: Some("bad/prefix-".to_string()),
            ..Default::default()
        };

        assert!(validate_create_name(&meta).is_err());
    }
}
