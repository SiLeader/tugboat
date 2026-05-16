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

fn default<T: Default + PartialEq>(t: &T) -> bool {
    *t == Default::default()
}

pub mod apiextensions {
    pub mod v1 {
        use crate::validators::{NamespaceProhibitedValidator, Validator};
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

        pub struct CrdSpecValidator;

        impl Validator<CustomResourceDefinition> for CrdSpecValidator {
            fn validate(&self, value: &CustomResourceDefinition) -> bool {
                let Some(spec) = value.spec.as_ref() else {
                    return false;
                };
                if spec.group.is_empty() || !matches!(spec.scope.as_str(), "Namespaced" | "Cluster")
                {
                    return false;
                }
                if RESERVED_GROUPS.contains(&spec.group.as_str()) {
                    return false;
                }
                if spec.versions.len() != 1 {
                    return false;
                }

                let version = &spec.versions[0];
                if version.name.is_empty() || !version.served || !version.storage {
                    return false;
                }

                let Some(names) = spec.names.as_ref() else {
                    return false;
                };
                if names.plural.is_empty() || names.kind.is_empty() {
                    return false;
                }

                let expected_name = format!("{}.{}", names.plural, spec.group);
                let actual_name = value
                    .object_meta
                    .as_ref()
                    .and_then(|metadata| metadata.name.as_deref())
                    .unwrap_or_default();
                if actual_name != expected_name {
                    return false;
                }

                if let Some(schema) = version.schema.as_ref()
                    && serde_json::from_str::<serde_json::Value>(&schema.open_api_v3_schema)
                        .is_err()
                {
                    return false;
                }

                true
            }
        }

        #[cfg(test)]
        mod tests {
            use super::{
                CustomResourceDefinition, CustomResourceDefinitionNames,
                CustomResourceDefinitionSpec, CustomResourceDefinitionVersion,
                CustomResourceValidation,
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
            }

            #[test]
            fn crd_name_must_match_plural_dot_group() {
                let mut crd = valid_crd();
                crd.object_meta.as_mut().unwrap().name = Some("wrong.example.com".to_string());

                assert!(!crd.validate());
            }

            #[test]
            fn crd_rejects_invalid_schema_json() {
                let mut crd = valid_crd();
                crd.spec.as_mut().unwrap().versions[0].schema = Some(CustomResourceValidation {
                    open_api_v3_schema: "{".to_string(),
                });

                assert!(!crd.validate());
            }

            #[test]
            fn crd_rejects_reserved_group() {
                let mut crd = valid_crd();
                crd.object_meta.as_mut().unwrap().name = Some("widgets.core".to_string());
                crd.spec.as_mut().unwrap().group = "core".to_string();

                assert!(!crd.validate());
            }
        }
    }
}

pub mod core {
    pub mod v1 {
        use crate::validators::{
            HasNodeAffinity, HasReclaimPolicy, HasVolumeBindingMode, NameValidator,
            NamespaceProhibitedValidator, NodeAffinityValidator, ReclaimPolicyValidator,
            ShipSchedulingValidator, Validator, VolumeBindingModeValidator,
        };
        use crate::{apply_resource, apply_validators, resource_api};

        include!(concat!(env!("OUT_DIR"), "/tugboat.core.v1.rs"));

        apply_resource!(ConfigMap, resource_api::CONFIG_MAP, namespaced);
        apply_resource!(Namespace, resource_api::NAMESPACE, cluster);
        apply_resource!(NetworkClass, resource_api::NETWORK_CLASS, namespaced);
        apply_resource!(
            ClusterNetworkClass,
            resource_api::CLUSTER_NETWORK_CLASS,
            cluster
        );
        apply_resource!(Node, resource_api::NODE, cluster);
        apply_resource!(PersistentVolume, resource_api::PERSISTENT_VOLUME, cluster);
        apply_resource!(
            PersistentVolumeClaim,
            resource_api::PERSISTENT_VOLUME_CLAIM,
            namespaced
        );
        apply_resource!(Secret, resource_api::SECRET, namespaced);
        apply_resource!(ServiceAccount, resource_api::SERVICE_ACCOUNT, namespaced);
        apply_resource!(RuntimeClass, resource_api::RUNTIME_CLASS, cluster);
        apply_resource!(StorageClass, resource_api::STORAGE_CLASS, cluster);
        apply_resource!(Ship, resource_api::SHIP, namespaced);
        apply_resource!(ShipClass, resource_api::SHIP_CLASS, cluster);
        apply_resource!(ShipSnapshot, resource_api::SHIP_SNAPSHOT, namespaced);

        apply_validators!(ConfigMap, validators NameValidator);
        apply_validators!(Namespace, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(NetworkClass, validators NameValidator);
        apply_validators!(ClusterNetworkClass, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(Node, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(
            PersistentVolume,
            validators NameValidator,
            NamespaceProhibitedValidator,
            NodeAffinityValidator
        );
        apply_validators!(
            PersistentVolumeClaim,
            validators NameValidator,
            PersistentVolumeClaimDataSourceValidator
        );
        apply_validators!(Secret, validators NameValidator);
        apply_validators!(ServiceAccount, validators NameValidator);
        apply_validators!(RuntimeClass, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(
            StorageClass,
            validators NameValidator,
            NamespaceProhibitedValidator,
            ReclaimPolicyValidator,
            VolumeBindingModeValidator
        );

        impl HasReclaimPolicy for StorageClass {
            fn reclaim_policy_value(&self) -> Option<&str> {
                self.spec.as_ref()?.reclaim_policy.as_deref()
            }
        }
        impl HasVolumeBindingMode for StorageClass {
            fn volume_binding_mode_value(&self) -> Option<&str> {
                self.spec.as_ref()?.volume_binding_mode.as_deref()
            }
        }
        impl HasNodeAffinity for PersistentVolume {
            fn node_affinity_value(&self) -> Option<&VolumeNodeAffinity> {
                self.spec.as_ref()?.node_affinity.as_ref()
            }
        }
        pub struct PersistentVolumeClaimDataSourceValidator;
        impl Validator<PersistentVolumeClaim> for PersistentVolumeClaimDataSourceValidator {
            fn validate(&self, value: &PersistentVolumeClaim) -> bool {
                let Some(data_source) = value
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.data_source.as_ref())
                else {
                    return true;
                };
                if data_source.name.is_empty() {
                    return false;
                }
                match data_source.kind.as_str() {
                    "VolumeSnapshot" => data_source.api_group == "snapshot",
                    "PersistentVolumeClaim" => data_source.api_group.is_empty(),
                    _ => false,
                }
            }
        }
        apply_validators!(Ship, validators NameValidator, ShipSchedulingValidator);
        apply_validators!(ShipClass, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(
            ShipSnapshot,
            validators NameValidator,
            ShipSnapshotSpecValidator,
            ShipSnapshotStatusValidator
        );

        pub struct ShipSnapshotSpecValidator;
        impl Validator<ShipSnapshot> for ShipSnapshotSpecValidator {
            fn validate(&self, value: &ShipSnapshot) -> bool {
                let Some(spec) = value.spec.as_ref() else {
                    return false;
                };
                if spec.ship_name.trim().is_empty() {
                    return false;
                }
                match spec.mode.as_deref() {
                    None | Some("Online") | Some("Offline") => {}
                    _ => return false,
                }
                true
            }
        }

        pub struct ShipSnapshotStatusValidator;
        impl Validator<ShipSnapshot> for ShipSnapshotStatusValidator {
            fn validate(&self, value: &ShipSnapshot) -> bool {
                let Some(status) = value.status.as_ref() else {
                    return true;
                };
                matches!(
                    status.phase.as_str(),
                    "Pending" | "Capturing" | "Ready" | "Failed"
                )
            }
        }

        #[cfg(test)]
        mod tests {
            use super::{
                ConfigMap, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm,
                PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimSpec,
                PersistentVolumeSpec, RuntimeClass, ShipSnapshot, ShipSnapshotSpec,
                ShipSnapshotStatus, StorageClass, StorageClassSpec, TypedLocalObjectReference,
                VolumeNodeAffinity,
            };
            use crate::manifests::meta::v1::ObjectMeta;
            use crate::validators::Validatable;

            #[test]
            fn configmap_with_valid_name_passes_validation() {
                let config_map = ConfigMap {
                    object_meta: Some(ObjectMeta {
                        name: Some("example-config".to_string()),
                        namespace: Some("default".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(config_map.validate());
            }

            #[test]
            fn configmap_with_invalid_name_fails_validation() {
                let config_map = ConfigMap {
                    object_meta: Some(ObjectMeta {
                        name: Some("Invalid_Config".to_string()),
                        namespace: Some("default".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(!config_map.validate());
            }

            #[test]
            fn runtimeclass_with_valid_name_passes_validation() {
                let rc = RuntimeClass {
                    object_meta: Some(ObjectMeta {
                        name: Some("qemu-kvm".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(rc.validate());
            }

            #[test]
            fn runtimeclass_with_invalid_name_fails_validation() {
                let rc = RuntimeClass {
                    object_meta: Some(ObjectMeta {
                        name: Some("Invalid_Name".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(!rc.validate());
            }

            #[test]
            fn runtimeclass_with_namespace_fails_validation() {
                let rc = RuntimeClass {
                    object_meta: Some(ObjectMeta {
                        name: Some("qemu-kvm".to_string()),
                        namespace: Some("default".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(!rc.validate());
            }

            #[test]
            fn storageclass_accepts_supported_volume_binding_modes() {
                for mode in ["Immediate", "WaitForFirstConsumer"] {
                    let storage_class = StorageClass {
                        object_meta: Some(ObjectMeta {
                            name: Some("fast".to_string()),
                            ..Default::default()
                        }),
                        spec: Some(StorageClassSpec {
                            provisioner: "example.csi.driver".to_string(),
                            volume_binding_mode: Some(mode.to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };

                    assert!(storage_class.validate(), "mode={mode}");
                }
            }

            #[test]
            fn storageclass_rejects_invalid_volume_binding_mode() {
                let storage_class = StorageClass {
                    object_meta: Some(ObjectMeta {
                        name: Some("fast".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(StorageClassSpec {
                        provisioner: "example.csi.driver".to_string(),
                        volume_binding_mode: Some("Delayed".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(!storage_class.validate());
            }

            #[test]
            fn persistent_volume_accepts_valid_node_affinity() {
                let pv = PersistentVolume {
                    object_meta: Some(ObjectMeta {
                        name: Some("pv-a".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(PersistentVolumeSpec {
                        node_affinity: Some(VolumeNodeAffinity {
                            required: Some(NodeSelector {
                                node_selector_terms: vec![NodeSelectorTerm {
                                    match_expressions: vec![NodeSelectorRequirement {
                                        key: "topology.tugboat.cloud/zone".to_string(),
                                        operator: "In".to_string(),
                                        values: vec!["us-east-a".to_string()],
                                    }],
                                    ..Default::default()
                                }],
                            }),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert!(pv.validate());
            }

            #[test]
            fn persistent_volume_rejects_invalid_node_affinity_requirements() {
                for (key, operator) in [("", "In"), ("topology.tugboat.cloud/zone", "Equals")] {
                    let pv = PersistentVolume {
                        object_meta: Some(ObjectMeta {
                            name: Some("pv-a".to_string()),
                            ..Default::default()
                        }),
                        spec: Some(PersistentVolumeSpec {
                            node_affinity: Some(VolumeNodeAffinity {
                                required: Some(NodeSelector {
                                    node_selector_terms: vec![NodeSelectorTerm {
                                        match_expressions: vec![NodeSelectorRequirement {
                                            key: key.to_string(),
                                            operator: operator.to_string(),
                                            values: vec!["us-east-a".to_string()],
                                        }],
                                        ..Default::default()
                                    }],
                                }),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };

                    assert!(!pv.validate(), "key={key} operator={operator}");
                }
            }

            #[test]
            fn persistent_volume_claim_accepts_supported_data_sources() {
                for (kind, api_group) in [
                    ("VolumeSnapshot", "snapshot"),
                    ("PersistentVolumeClaim", ""),
                ] {
                    let pvc = PersistentVolumeClaim {
                        object_meta: Some(ObjectMeta {
                            name: Some("restore-target".to_string()),
                            namespace: Some("default".to_string()),
                            ..Default::default()
                        }),
                        spec: Some(PersistentVolumeClaimSpec {
                            data_source: Some(TypedLocalObjectReference {
                                api_group: api_group.to_string(),
                                kind: kind.to_string(),
                                name: "source".to_string(),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };

                    assert!(pvc.validate(), "kind={kind}");
                }
            }

            #[test]
            fn persistent_volume_claim_rejects_unsupported_data_sources() {
                for (kind, api_group, name) in [
                    ("VolumeSnapshot", "", "snap-a"),
                    ("PersistentVolumeClaim", "snapshot", "pvc-a"),
                    ("ConfigMap", "", "config"),
                    ("VolumeSnapshot", "snapshot", ""),
                ] {
                    let pvc = PersistentVolumeClaim {
                        object_meta: Some(ObjectMeta {
                            name: Some("restore-target".to_string()),
                            namespace: Some("default".to_string()),
                            ..Default::default()
                        }),
                        spec: Some(PersistentVolumeClaimSpec {
                            data_source: Some(TypedLocalObjectReference {
                                api_group: api_group.to_string(),
                                kind: kind.to_string(),
                                name: name.to_string(),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };

                    assert!(
                        !pvc.validate(),
                        "kind={kind} api_group={api_group} name={name}"
                    );
                }
            }

            fn ship_snapshot_with_spec(
                name: &str,
                ship_name: &str,
                mode: Option<&str>,
            ) -> ShipSnapshot {
                ShipSnapshot {
                    object_meta: Some(ObjectMeta {
                        name: Some(name.to_string()),
                        namespace: Some("default".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(ShipSnapshotSpec {
                        ship_name: ship_name.to_string(),
                        mode: mode.map(str::to_string),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }

            #[test]
            fn ship_snapshot_accepts_valid_modes() {
                for mode in [None, Some("Online"), Some("Offline")] {
                    let snap = ship_snapshot_with_spec("snap-a", "ship-a", mode);
                    assert!(snap.validate(), "mode={mode:?}");
                }
            }

            #[test]
            fn ship_snapshot_rejects_unknown_mode() {
                let snap = ship_snapshot_with_spec("snap-a", "ship-a", Some("Fast"));
                assert!(!snap.validate());
            }

            #[test]
            fn ship_snapshot_rejects_empty_ship_name() {
                let snap = ship_snapshot_with_spec("snap-a", "  ", None);
                assert!(!snap.validate());
            }

            #[test]
            fn ship_snapshot_status_phase_must_be_known() {
                let mut snap = ship_snapshot_with_spec("snap-a", "ship-a", None);
                snap.status = Some(ShipSnapshotStatus {
                    phase: "Ready".to_string(),
                    ..Default::default()
                });
                assert!(snap.validate());

                snap.status = Some(ShipSnapshotStatus {
                    phase: "Bogus".to_string(),
                    ..Default::default()
                });
                assert!(!snap.validate());
            }
        }
    }
}

pub mod apps {
    pub mod v1 {
        use crate::validators::NameValidator;
        use crate::{apply_resource, apply_validators, resource_api};

        include!(concat!(env!("OUT_DIR"), "/tugboat.apps.v1.rs"));

        apply_resource!(Deployment, resource_api::DEPLOYMENT, namespaced);
        apply_resource!(ReplicaSet, resource_api::REPLICA_SET, namespaced);
        apply_resource!(Fleet, resource_api::FLEET, namespaced);

        apply_validators!(Deployment, validators NameValidator);
        apply_validators!(ReplicaSet, validators NameValidator);
        apply_validators!(Fleet, validators NameValidator);
    }
}

pub mod authorization {
    pub mod v1 {
        use crate::validators::{NameValidator, NamespaceProhibitedValidator};
        use crate::{apply_resource, apply_validators, resource_api};

        include!(concat!(env!("OUT_DIR"), "/tugboat.authorization.v1.rs"));

        apply_resource!(ClusterRole, resource_api::CLUSTER_ROLE, cluster);
        apply_resource!(
            ClusterRoleBinding,
            resource_api::CLUSTER_ROLE_BINDING,
            cluster
        );
        apply_resource!(Role, resource_api::ROLE, namespaced);
        apply_resource!(RoleBinding, resource_api::ROLE_BINDING, namespaced);

        apply_validators!(
            ClusterRole,
            validators NameValidator,
            NamespaceProhibitedValidator
        );
        apply_validators!(
            ClusterRoleBinding,
            validators NameValidator,
            NamespaceProhibitedValidator
        );
        apply_validators!(Role, validators NameValidator);
        apply_validators!(RoleBinding, validators NameValidator);
    }
}

pub mod coordination {
    pub mod v1 {
        use crate::validators::NameValidator;
        use crate::{apply_resource, apply_validators, resource_api};

        include!(concat!(env!("OUT_DIR"), "/tugboat.coordination.v1.rs"));

        apply_resource!(Lease, resource_api::LEASE, namespaced);

        apply_validators!(Lease, validators NameValidator);
    }
}

pub mod snapshot {
    pub mod v1 {
        use crate::validators::{NameValidator, NamespaceProhibitedValidator, Validator};
        use crate::{apply_resource, apply_validators, resource_api};

        include!(concat!(env!("OUT_DIR"), "/tugboat.snapshot.v1.rs"));

        apply_resource!(VolumeSnapshot, resource_api::VOLUME_SNAPSHOT, namespaced);
        apply_resource!(
            VolumeSnapshotContent,
            resource_api::VOLUME_SNAPSHOT_CONTENT,
            cluster
        );
        apply_resource!(
            VolumeSnapshotClass,
            resource_api::VOLUME_SNAPSHOT_CLASS,
            cluster
        );

        apply_validators!(
            VolumeSnapshot,
            validators NameValidator,
            VolumeSnapshotSourceValidator
        );
        apply_validators!(
            VolumeSnapshotContent,
            validators NameValidator,
            NamespaceProhibitedValidator,
            VolumeSnapshotContentSpecValidator
        );
        apply_validators!(
            VolumeSnapshotClass,
            validators NameValidator,
            NamespaceProhibitedValidator,
            VolumeSnapshotClassSpecValidator
        );

        pub struct VolumeSnapshotSourceValidator;
        impl Validator<VolumeSnapshot> for VolumeSnapshotSourceValidator {
            fn validate(&self, value: &VolumeSnapshot) -> bool {
                let Some(source) = value.spec.as_ref().and_then(|spec| spec.source.as_ref()) else {
                    return false;
                };
                exactly_one_set(
                    source.persistent_volume_claim_name.as_ref(),
                    source.volume_snapshot_content_name.as_ref(),
                )
            }
        }

        pub struct VolumeSnapshotContentSpecValidator;
        impl Validator<VolumeSnapshotContent> for VolumeSnapshotContentSpecValidator {
            fn validate(&self, value: &VolumeSnapshotContent) -> bool {
                let Some(spec) = value.spec.as_ref() else {
                    return false;
                };
                if !valid_deletion_policy(&spec.deletion_policy) {
                    return false;
                }
                let Some(source) = spec.source.as_ref() else {
                    return false;
                };
                exactly_one_set(
                    source.volume_handle.as_ref(),
                    source.snapshot_handle.as_ref(),
                )
            }
        }

        pub struct VolumeSnapshotClassSpecValidator;
        impl Validator<VolumeSnapshotClass> for VolumeSnapshotClassSpecValidator {
            fn validate(&self, value: &VolumeSnapshotClass) -> bool {
                value
                    .spec
                    .as_ref()
                    .is_some_and(|spec| valid_deletion_policy(&spec.deletion_policy))
            }
        }

        fn exactly_one_set(left: Option<&String>, right: Option<&String>) -> bool {
            left.is_some() ^ right.is_some()
        }

        fn valid_deletion_policy(value: &str) -> bool {
            matches!(value, "Retain" | "Delete")
        }

        #[cfg(test)]
        mod tests {
            use super::{
                VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotClassSpec,
                VolumeSnapshotContent, VolumeSnapshotContentSource, VolumeSnapshotContentSpec,
                VolumeSnapshotSource,
            };
            use crate::manifests::meta::v1::ObjectMeta;
            use crate::validators::Validatable;

            #[test]
            fn volume_snapshot_requires_exactly_one_source() {
                for (pvc, content, valid) in [
                    (Some("claim-a"), None, true),
                    (None, Some("content-a"), true),
                    (None, None, false),
                    (Some("claim-a"), Some("content-a"), false),
                ] {
                    let snapshot = VolumeSnapshot {
                        object_meta: Some(ObjectMeta {
                            name: Some("snap-a".to_string()),
                            namespace: Some("default".to_string()),
                            ..Default::default()
                        }),
                        spec: Some(super::VolumeSnapshotSpec {
                            source: Some(VolumeSnapshotSource {
                                persistent_volume_claim_name: pvc.map(str::to_string),
                                volume_snapshot_content_name: content.map(str::to_string),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };

                    assert_eq!(
                        snapshot.validate(),
                        valid,
                        "pvc={pvc:?} content={content:?}"
                    );
                }
            }

            #[test]
            fn volume_snapshot_content_validates_source_policy_and_namespace() {
                let content = VolumeSnapshotContent {
                    object_meta: Some(ObjectMeta {
                        name: Some("content-a".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(VolumeSnapshotContentSpec {
                        deletion_policy: "Delete".to_string(),
                        source: Some(VolumeSnapshotContentSource {
                            volume_handle: Some("vol-a".to_string()),
                            snapshot_handle: None,
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                assert!(content.validate());

                let mut both_sources = content.clone();
                both_sources.spec.as_mut().unwrap().source = Some(VolumeSnapshotContentSource {
                    volume_handle: Some("vol-a".to_string()),
                    snapshot_handle: Some("snap-a".to_string()),
                });
                assert!(!both_sources.validate());

                let mut invalid_policy = content.clone();
                invalid_policy.spec.as_mut().unwrap().deletion_policy = "Archive".to_string();
                assert!(!invalid_policy.validate());

                let mut namespaced = content;
                namespaced.object_meta.as_mut().unwrap().namespace = Some("default".to_string());
                assert!(!namespaced.validate());
            }

            #[test]
            fn volume_snapshot_class_validates_policy_and_namespace() {
                for policy in ["Retain", "Delete"] {
                    let class = VolumeSnapshotClass {
                        object_meta: Some(ObjectMeta {
                            name: Some("snapclass".to_string()),
                            ..Default::default()
                        }),
                        spec: Some(VolumeSnapshotClassSpec {
                            deletion_policy: policy.to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    assert!(class.validate(), "policy={policy}");
                }

                let invalid = VolumeSnapshotClass {
                    object_meta: Some(ObjectMeta {
                        name: Some("snapclass".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(VolumeSnapshotClassSpec {
                        deletion_policy: "Archive".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                assert!(!invalid.validate());
            }
        }
    }
}

pub mod meta {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/tugboat.meta.v1.rs"));

        impl crate::Resource for CustomResourceObject {
            fn type_meta() -> TypeMeta {
                TypeMeta {
                    api_version: Some("meta/v1".to_string()),
                    kind: Some("CustomResourceObject".to_string()),
                }
            }
        }

        impl crate::ObjectMetaResource for CustomResourceObject {
            fn object_meta(&self) -> &Option<ObjectMeta> {
                &self.object_meta
            }

            fn object_meta_mut(&mut self) -> &mut Option<ObjectMeta> {
                &mut self.object_meta
            }
        }

        impl crate::SetTypeMeta for CustomResourceObject {
            fn set_type_meta(&mut self, type_meta: TypeMeta) {
                self.type_meta = Some(type_meta);
            }
        }

        impl Time {
            pub fn now() -> Self {
                let now = std::time::SystemTime::now();
                let duration = now
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();

                let seconds = duration.as_secs() as i64;
                let nanos = duration.subsec_nanos() as i32;
                Time { seconds, nanos }
            }
        }

        impl serde::Serialize for Time {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                if let Some(dt) =
                    ::chrono::DateTime::from_timestamp(self.seconds, self.nanos as u32)
                {
                    serializer.serialize_str(&dt.to_rfc3339())
                } else {
                    use serde::ser::Error;
                    Err(S::Error::custom("invalid timestamp"))
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for Time {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                let dt =
                    ::chrono::DateTime::parse_from_rfc3339(&s).map_err(serde::de::Error::custom)?;
                let dt_utc = dt.with_timezone(&::chrono::Utc);
                Ok(Time {
                    seconds: dt_utc.timestamp(),
                    nanos: dt_utc.timestamp_subsec_nanos() as i32,
                })
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn test_time_serialization() {
                let t = Time {
                    seconds: 1678896000, // 2023-03-15T16:00:00Z
                    nanos: 0,
                };
                let json = serde_json::to_string(&t).unwrap();
                // Depending on chrono version/timezone, output might vary slightly, but should be RFC3339
                assert!(json.contains("2023-03-15T16:00:00"));

                let t2: Time = serde_json::from_str(&json).unwrap();
                assert_eq!(t.seconds, t2.seconds);
                assert_eq!(t.nanos, t2.nanos);
            }

            #[test]
            fn test_time_deserialization() {
                let json = "\"2023-03-15T16:00:00.123456Z\"";
                let t: Time = serde_json::from_str(json).unwrap();
                assert_eq!(t.seconds, 1678896000);
                assert_eq!(t.nanos, 123456000);
            }
        }
    }
}
