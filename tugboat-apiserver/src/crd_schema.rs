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

use std::fmt::{Debug, Formatter};

use tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinition;

pub(crate) const MAX_VALIDATION_ERRORS: usize = 20;

pub struct CompiledSchema(jsonschema::Validator);

impl CompiledSchema {
    pub(crate) fn validate(&self, value: &serde_json::Value) -> Result<(), Vec<SchemaViolation>> {
        let violations = self
            .0
            .iter_errors(value)
            .take(MAX_VALIDATION_ERRORS)
            .map(|err| SchemaViolation {
                field: instance_path_to_field(err.instance_path().to_string()),
                message: err.to_string(),
            })
            .collect::<Vec<_>>();

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

impl Debug for CompiledSchema {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledSchema").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SchemaViolation {
    pub(crate) field: String,
    pub(crate) message: String,
}

pub(crate) fn parse_schema(raw: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(raw).map_err(|err| err.to_string())
}

pub(crate) fn compile_schema(schema: &serde_json::Value) -> Result<CompiledSchema, String> {
    jsonschema::draft7::new(schema)
        .map(CompiledSchema)
        .map_err(|err| err.to_string())
}

pub(crate) fn validate_crd_schemas(crd: &CustomResourceDefinition) -> Result<(), String> {
    let Some(spec) = crd.spec.as_ref() else {
        return Ok(());
    };

    for version in &spec.versions {
        let Some(schema) = version.schema.as_ref() else {
            continue;
        };
        if schema.open_api_v3_schema.trim().is_empty() {
            continue;
        }
        let schema_json = parse_schema(&schema.open_api_v3_schema)?;
        compile_schema(&schema_json)?;
    }

    Ok(())
}

fn instance_path_to_field(path: String) -> String {
    if path.is_empty() {
        return ".".to_string();
    }

    path.trim_start_matches('/')
        .split('/')
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::{compile_schema, parse_schema};

    #[test]
    fn compiles_valid_draft7_schema() {
        let schema = parse_schema(r#"{"type":"object"}"#).expect("schema should parse");

        assert!(compile_schema(&schema).is_ok());
    }

    #[test]
    fn rejects_invalid_schema_document() {
        let schema = parse_schema(r#"{"type":"not-a-json-schema-type"}"#).expect("schema parses");

        assert!(compile_schema(&schema).is_err());
    }
}
