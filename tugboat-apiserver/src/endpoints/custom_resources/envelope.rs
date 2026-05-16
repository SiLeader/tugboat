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

use super::metadata::inject_resource_version;
use crate::crd_registry::CrdEntry;
use crate::data::StatusResponse;
use tugboat_resource_store::ContentData;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::meta::v1::{CustomResourceObject, ObjectMeta, TypeMeta};

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

pub(super) fn envelope_from_value(
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

pub(super) fn enforce_type_meta(
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
