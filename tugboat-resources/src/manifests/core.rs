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
            ConfigMap, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm, PersistentVolume,
            PersistentVolumeClaim, PersistentVolumeClaimSpec, PersistentVolumeSpec, RuntimeClass,
            ShipSnapshot, ShipSnapshotSpec, ShipSnapshotStatus, StorageClass, StorageClassSpec,
            TypedLocalObjectReference, VolumeNodeAffinity,
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
