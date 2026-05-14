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

pub mod core {
    pub mod v1 {
        use crate::validators::{
            HasNodeAffinity, HasReclaimPolicy, HasVolumeBindingMode, NameValidator,
            NamespaceProhibitedValidator, NodeAffinityValidator, ReclaimPolicyValidator,
            VolumeBindingModeValidator,
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
        apply_validators!(PersistentVolumeClaim, validators NameValidator);
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
        apply_validators!(Ship, validators NameValidator);
        apply_validators!(ShipClass, validators NameValidator, NamespaceProhibitedValidator);

        #[cfg(test)]
        mod tests {
            use super::{
                ConfigMap, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm,
                PersistentVolume, PersistentVolumeSpec, RuntimeClass, StorageClass,
                StorageClassSpec, VolumeNodeAffinity,
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

pub mod meta {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/tugboat.meta.v1.rs"));

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
