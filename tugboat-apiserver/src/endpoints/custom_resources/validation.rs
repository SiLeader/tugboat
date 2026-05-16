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

pub(super) fn validate_against_schema(
    entry: &CrdEntry,
    value: &serde_json::Value,
) -> Result<(), Box<StatusResponse>> {
    let Some(schema) = entry.version.compiled_schema.as_ref() else {
        return Ok(());
    };

    let violations = match schema.validate(value) {
        Ok(()) => return Ok(()),
        Err(violations) => violations,
    };
    let message = format!(
        "{}.{} \"{}\" is invalid: {}",
        entry.kind,
        entry.group,
        value
            .get("metadata")
            .and_then(|metadata| metadata.get("name"))
            .and_then(|name| name.as_str())
            .unwrap_or("<unknown>"),
        violations
            .iter()
            .map(|violation| format!("{}: {}", violation.field, violation.message))
            .collect::<Vec<_>>()
            .join("; ")
    );
    let causes = violations
        .iter()
        .map(|violation| {
            serde_json::json!({
                "reason": "FieldValueInvalid",
                "message": violation.message,
                "field": violation.field,
            })
        })
        .collect::<Vec<_>>();

    Err(Box::new(StatusResponse::invalid(
        message,
        Some(serde_json::json!({
            "group": entry.group,
            "kind": entry.kind,
            "name": value
                .get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(|name| name.as_str()),
            "causes": causes,
        })),
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::validate_against_schema;
    use crate::crd_registry::{CrdEntry, CrdScope, CrdVersionInfo};
    use crate::crd_schema::{compile_schema, parse_schema};
    use serde_json::json;

    fn entry(schema: Option<serde_json::Value>) -> CrdEntry {
        let (schema_json, compiled_schema) = schema
            .map(|schema| {
                let schema = Arc::new(schema);
                let compiled = Arc::new(compile_schema(&schema).expect("schema should compile"));
                (Some(schema), Some(compiled))
            })
            .unwrap_or((None, None));

        CrdEntry {
            group: "example.com".to_string(),
            plural: "widgets".to_string(),
            singular: "widget".to_string(),
            kind: "Widget".to_string(),
            list_kind: "WidgetList".to_string(),
            scope: CrdScope::Namespaced,
            version: CrdVersionInfo {
                name: "v1".to_string(),
                served: true,
                storage: true,
                schema_json,
                compiled_schema,
                status_subresource: false,
            },
        }
    }

    fn instance() -> serde_json::Value {
        json!({
            "apiVersion": "example.com/v1",
            "kind": "Widget",
            "metadata": {"name": "demo"},
            "spec": {"replicas": 2, "image": "demo:v1"}
        })
    }

    fn schema(raw: &str) -> serde_json::Value {
        parse_schema(raw).expect("schema should parse")
    }

    #[test]
    fn missing_schema_accepts_any_json() {
        assert!(validate_against_schema(&entry(None), &json!({"anything": true})).is_ok());
    }

    #[test]
    fn valid_instance_passes_schema_validation() {
        let schema = schema(
            r#"{
                "type": "object",
                "required": ["spec"],
                "properties": {
                    "spec": {
                        "type": "object",
                        "required": ["replicas", "image"],
                        "properties": {
                            "replicas": {"type": "integer", "minimum": 1},
                            "image": {"type": "string"}
                        }
                    }
                }
            }"#,
        );

        assert!(validate_against_schema(&entry(Some(schema)), &instance()).is_ok());
    }

    #[test]
    fn required_field_missing_fails() {
        let schema = schema(
            r#"{
                "type": "object",
                "required": ["spec"],
                "properties": {"spec": {"type": "object", "required": ["image"]}}
            }"#,
        );
        let mut value = instance();
        value["spec"].as_object_mut().unwrap().remove("image");

        let err = validate_against_schema(&entry(Some(schema)), &value).unwrap_err();
        assert!(format!("{err:?}").contains("image"));
    }

    #[test]
    fn type_mismatch_fails() {
        let schema = schema(
            r#"{
                "type": "object",
                "properties": {"spec": {"type": "object", "properties": {"replicas": {"type": "integer"}}}}
            }"#,
        );
        let mut value = instance();
        value["spec"]["replicas"] = json!("two");

        assert!(validate_against_schema(&entry(Some(schema)), &value).is_err());
    }

    #[test]
    fn minimum_violation_fails() {
        let schema = schema(
            r#"{
                "type": "object",
                "properties": {"spec": {"type": "object", "properties": {"replicas": {"type": "integer", "minimum": 1}}}}
            }"#,
        );
        let mut value = instance();
        value["spec"]["replicas"] = json!(0);

        assert!(validate_against_schema(&entry(Some(schema)), &value).is_err());
    }

    #[test]
    fn additional_properties_false_rejects_unknown_fields() {
        let schema = schema(
            r#"{
                "type": "object",
                "properties": {
                    "apiVersion": {"type": "string"},
                    "kind": {"type": "string"},
                    "metadata": {"type": "object"},
                    "spec": {"type": "object"}
                },
                "additionalProperties": false
            }"#,
        );
        let mut value = instance();
        value["unexpected"] = json!(true);

        assert!(validate_against_schema(&entry(Some(schema)), &value).is_err());
    }
}
