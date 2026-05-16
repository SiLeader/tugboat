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

pub mod v1 {
    use crate::validators::{NameValidator, NamespaceProhibitedValidator, Validator};
    use crate::{apply_resource, apply_validators, resource_api};

    include!(concat!(env!("OUT_DIR"), "/tugboat.apiextensions.v1.rs"));

    apply_resource!(
        CustomResourceDefinition,
        resource_api::CUSTOM_RESOURCE_DEFINITION,
        cluster
    );

    apply_validators!(
        CustomResourceDefinition,
        validators NamespaceProhibitedValidator, CrdSpecValidator
    );

    pub const RESERVED_GROUPS: &[&str] = &[
        "core",
        "apps",
        "authorization",
        "coordination",
        "snapshot",
        "apiextensions",
    ];

    /// Detailed reasons a CustomResourceDefinition can fail spec validation.
    /// Used by the apiserver to surface field-level causes in error responses
    /// rather than a single opaque "invalid" boolean.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum CrdSpecValidationError {
        MissingSpec,
        UnsupportedScope(String),
        InvalidGroup(String),
        ReservedGroup(String),
        UnsupportedVersionCount(usize),
        InvalidVersionName(String),
        VersionNotServed(String),
        VersionNotStorage(String),
        MissingNames,
        InvalidPlural(String),
        InvalidSingular(String),
        InvalidKind(String),
        InvalidListKind(String),
        NameMismatch { expected: String, actual: String },
        InvalidOpenApiSchemaJson(String),
    }

    impl std::fmt::Display for CrdSpecValidationError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::MissingSpec => write!(f, "spec is required"),
                Self::UnsupportedScope(scope) => write!(
                    f,
                    "spec.scope must be \"Cluster\" or \"Namespaced\", got \"{scope}\""
                ),
                Self::InvalidGroup(group) => {
                    write!(f, "spec.group \"{group}\" is not a valid DNS subdomain")
                }
                Self::ReservedGroup(group) => write!(
                    f,
                    "spec.group \"{group}\" is reserved for built-in resources"
                ),
                Self::UnsupportedVersionCount(count) => write!(
                    f,
                    "spec.versions must contain exactly one version (got {count})"
                ),
                Self::InvalidVersionName(name) => {
                    write!(f, "spec.versions[0].name \"{name}\" is invalid")
                }
                Self::VersionNotServed(name) => write!(
                    f,
                    "spec.versions[0].served must be true for the only version (\"{name}\")"
                ),
                Self::VersionNotStorage(name) => write!(
                    f,
                    "spec.versions[0].storage must be true for the only version (\"{name}\")"
                ),
                Self::MissingNames => write!(f, "spec.names is required"),
                Self::InvalidPlural(plural) => {
                    write!(f, "spec.names.plural \"{plural}\" is invalid")
                }
                Self::InvalidSingular(singular) => {
                    write!(f, "spec.names.singular \"{singular}\" is invalid")
                }
                Self::InvalidKind(kind) => {
                    write!(f, "spec.names.kind \"{kind}\" must be UpperCamelCase")
                }
                Self::InvalidListKind(list_kind) => {
                    write!(
                        f,
                        "spec.names.listKind \"{list_kind}\" must be UpperCamelCase"
                    )
                }
                Self::NameMismatch { expected, actual } => {
                    write!(f, "metadata.name must be \"{expected}\" (got \"{actual}\")")
                }
                Self::InvalidOpenApiSchemaJson(message) => write!(
                    f,
                    "spec.versions[0].schema.openApiV3Schema is not valid JSON: {message}"
                ),
            }
        }
    }

    /// Validate the full CRD spec and return the first failure, if any.
    /// `CrdSpecValidator` (boolean) wraps this for the macro-driven validator
    /// chain; the apiserver calls this directly to render field-level causes.
    pub fn validate_crd_spec(crd: &CustomResourceDefinition) -> Result<(), CrdSpecValidationError> {
        let Some(spec) = crd.spec.as_ref() else {
            return Err(CrdSpecValidationError::MissingSpec);
        };
        if !matches!(spec.scope.as_str(), "Namespaced" | "Cluster") {
            return Err(CrdSpecValidationError::UnsupportedScope(spec.scope.clone()));
        }
        if !NameValidator::is_valid_dns_subdomain(&spec.group) {
            return Err(CrdSpecValidationError::InvalidGroup(spec.group.clone()));
        }
        if RESERVED_GROUPS.contains(&spec.group.as_str()) {
            return Err(CrdSpecValidationError::ReservedGroup(spec.group.clone()));
        }
        if spec.versions.len() != 1 {
            return Err(CrdSpecValidationError::UnsupportedVersionCount(
                spec.versions.len(),
            ));
        }

        let version = &spec.versions[0];
        if !NameValidator::is_valid_name(&version.name) {
            return Err(CrdSpecValidationError::InvalidVersionName(
                version.name.clone(),
            ));
        }
        if !version.served {
            return Err(CrdSpecValidationError::VersionNotServed(
                version.name.clone(),
            ));
        }
        if !version.storage {
            return Err(CrdSpecValidationError::VersionNotStorage(
                version.name.clone(),
            ));
        }

        let Some(names) = spec.names.as_ref() else {
            return Err(CrdSpecValidationError::MissingNames);
        };
        if !NameValidator::is_valid_name(&names.plural) {
            return Err(CrdSpecValidationError::InvalidPlural(names.plural.clone()));
        }
        if !NameValidator::is_valid_name(&names.singular) {
            return Err(CrdSpecValidationError::InvalidSingular(
                names.singular.clone(),
            ));
        }
        if !NameValidator::is_valid_kind_name(&names.kind) {
            return Err(CrdSpecValidationError::InvalidKind(names.kind.clone()));
        }
        if !names.list_kind.is_empty() && !NameValidator::is_valid_kind_name(&names.list_kind) {
            return Err(CrdSpecValidationError::InvalidListKind(
                names.list_kind.clone(),
            ));
        }

        let expected_name = format!("{}.{}", names.plural, spec.group);
        let actual_name = crd
            .object_meta
            .as_ref()
            .and_then(|metadata| metadata.name.as_deref())
            .unwrap_or_default();
        if actual_name != expected_name {
            return Err(CrdSpecValidationError::NameMismatch {
                expected: expected_name,
                actual: actual_name.to_string(),
            });
        }

        if let Some(schema) = version.schema.as_ref()
            && let Err(err) = serde_json::from_str::<serde_json::Value>(&schema.open_api_v3_schema)
        {
            return Err(CrdSpecValidationError::InvalidOpenApiSchemaJson(
                err.to_string(),
            ));
        }

        Ok(())
    }

    pub struct CrdSpecValidator;

    impl Validator<CustomResourceDefinition> for CrdSpecValidator {
        fn validate(&self, value: &CustomResourceDefinition) -> bool {
            validate_crd_spec(value).is_ok()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            CrdSpecValidationError, CustomResourceDefinition, CustomResourceDefinitionNames,
            CustomResourceDefinitionSpec, CustomResourceDefinitionVersion,
            CustomResourceValidation, validate_crd_spec,
        };
        use crate::manifests::meta::v1::ObjectMeta;
        use crate::validators::Validatable;

        fn valid_crd() -> CustomResourceDefinition {
            CustomResourceDefinition {
                object_meta: Some(ObjectMeta {
                    name: Some("widgets.example.com".to_string()),
                    ..Default::default()
                }),
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
                            open_api_v3_schema: r#"{"type":"object"}"#.to_string(),
                        }),
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            }
        }

        #[test]
        fn crd_with_valid_spec_passes_validation() {
            assert!(valid_crd().validate());
            assert!(validate_crd_spec(&valid_crd()).is_ok());
        }

        #[test]
        fn crd_name_must_match_plural_dot_group() {
            let mut crd = valid_crd();
            crd.object_meta.as_mut().unwrap().name = Some("wrong.example.com".to_string());

            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::NameMismatch { .. })
            ));
        }

        #[test]
        fn crd_rejects_invalid_schema_json() {
            let mut crd = valid_crd();
            crd.spec.as_mut().unwrap().versions[0].schema = Some(CustomResourceValidation {
                open_api_v3_schema: "{".to_string(),
            });

            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::InvalidOpenApiSchemaJson(_))
            ));
        }

        #[test]
        fn crd_rejects_reserved_group() {
            let mut crd = valid_crd();
            crd.object_meta.as_mut().unwrap().name = Some("widgets.core".to_string());
            crd.spec.as_mut().unwrap().group = "core".to_string();

            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::ReservedGroup(group)) if group == "core"
            ));
        }

        #[test]
        fn crd_rejects_missing_names() {
            let mut crd = valid_crd();
            crd.spec.as_mut().unwrap().names = None;

            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::MissingNames)
            ));
        }

        #[test]
        fn crd_rejects_unsupported_scope() {
            let mut crd = valid_crd();
            crd.spec.as_mut().unwrap().scope = "Namespace".to_string();

            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::UnsupportedScope(scope)) if scope == "Namespace"
            ));
        }

        #[test]
        fn crd_rejects_multiple_versions_for_minimal_implementation() {
            let mut crd = valid_crd();
            let version = crd.spec.as_ref().unwrap().versions[0].clone();
            crd.spec.as_mut().unwrap().versions.push(version);

            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::UnsupportedVersionCount(2))
            ));
        }

        #[test]
        fn crd_rejects_unserved_or_nonstorage_version() {
            let mut unserved = valid_crd();
            unserved.spec.as_mut().unwrap().versions[0].served = false;
            assert!(!unserved.validate());
            assert!(matches!(
                validate_crd_spec(&unserved),
                Err(CrdSpecValidationError::VersionNotServed(_))
            ));

            let mut nonstorage = valid_crd();
            nonstorage.spec.as_mut().unwrap().versions[0].storage = false;
            assert!(!nonstorage.validate());
            assert!(matches!(
                validate_crd_spec(&nonstorage),
                Err(CrdSpecValidationError::VersionNotStorage(_))
            ));
        }

        #[test]
        fn crd_rejects_group_that_is_not_a_dns_subdomain() {
            let mut crd = valid_crd();
            crd.object_meta.as_mut().unwrap().name = Some("widgets.Example.com".to_string());
            crd.spec.as_mut().unwrap().group = "Example.com".to_string();
            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::InvalidGroup(_))
            ));
        }

        #[test]
        fn crd_rejects_plural_with_uppercase() {
            let mut crd = valid_crd();
            crd.object_meta.as_mut().unwrap().name = Some("Widgets.example.com".to_string());
            crd.spec.as_mut().unwrap().names.as_mut().unwrap().plural = "Widgets".to_string();
            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::InvalidPlural(_))
            ));
        }

        #[test]
        fn crd_rejects_kind_with_lowercase_initial() {
            let mut crd = valid_crd();
            crd.spec.as_mut().unwrap().names.as_mut().unwrap().kind = "widget".to_string();
            assert!(!crd.validate());
            assert!(matches!(
                validate_crd_spec(&crd),
                Err(CrdSpecValidationError::InvalidKind(_))
            ));
        }
    }
}
