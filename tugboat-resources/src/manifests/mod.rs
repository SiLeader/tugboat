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
            HasReclaimPolicy, NameValidator, NamespaceProhibitedValidator, ReclaimPolicyValidator,
        };
        use crate::{apply_resource, apply_validators};

        include!(concat!(env!("OUT_DIR"), "/tugboat.core.v1.rs"));

        apply_resource!(
            ConfigMap,
            "core",
            "v1",
            "configmaps",
            "configmap",
            namespaced
        );
        apply_resource!(Namespace, "core", "v1", "namespaces", "namespace", cluster);
        apply_resource!(
            NetworkClass,
            "core",
            "v1",
            "networkclasses",
            "networkclass",
            namespaced
        );
        apply_resource!(
            ClusterNetworkClass,
            "core",
            "v1",
            "clusternetworkclasses",
            "clusternetworkclass",
            cluster
        );
        apply_resource!(Node, "core", "v1", "nodes", "node", cluster);
        apply_resource!(
            PersistentVolume,
            "core",
            "v1",
            "persistentvolumes",
            "persistentvolume",
            cluster
        );
        apply_resource!(
            PersistentVolumeClaim,
            "core",
            "v1",
            "persistentvolumeclaims",
            "persistentvolumeclaim",
            namespaced
        );
        apply_resource!(Secret, "core", "v1", "secrets", "secret", namespaced);
        apply_resource!(
            ServiceAccount,
            "core",
            "v1",
            "serviceaccounts",
            "serviceaccount",
            namespaced
        );
        apply_resource!(
            RuntimeClass,
            "core",
            "v1",
            "runtimeclasses",
            "runtimeclass",
            cluster
        );
        apply_resource!(
            StorageClass,
            "core",
            "v1",
            "storageclasses",
            "storageclass",
            cluster
        );
        apply_resource!(Ship, "core", "v1", "ships", "ship", namespaced);
        apply_resource!(ShipClass, "core", "v1", "shipclasses", "shipclass", cluster);

        apply_validators!(ConfigMap, validators NameValidator);
        apply_validators!(Namespace, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(NetworkClass, validators NameValidator);
        apply_validators!(ClusterNetworkClass, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(Node, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(PersistentVolume, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(PersistentVolumeClaim, validators NameValidator);
        apply_validators!(Secret, validators NameValidator);
        apply_validators!(ServiceAccount, validators NameValidator);
        apply_validators!(RuntimeClass, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(StorageClass, validators NameValidator, NamespaceProhibitedValidator, ReclaimPolicyValidator);

        impl HasReclaimPolicy for StorageClass {
            fn reclaim_policy_value(&self) -> Option<&str> {
                self.spec.as_ref()?.reclaim_policy.as_deref()
            }
        }
        apply_validators!(Ship, validators NameValidator);
        apply_validators!(ShipClass, validators NameValidator, NamespaceProhibitedValidator);

        #[cfg(test)]
        mod tests {
            use super::{ConfigMap, RuntimeClass};
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
        }
    }
}

pub mod apps {
    pub mod v1 {
        use crate::validators::NameValidator;
        use crate::{apply_resource, apply_validators};

        include!(concat!(env!("OUT_DIR"), "/tugboat.apps.v1.rs"));

        apply_resource!(
            Deployment,
            "apps",
            "v1",
            "deployments",
            "deployment",
            namespaced
        );
        apply_resource!(
            ReplicaSet,
            "apps",
            "v1",
            "replicasets",
            "replicaset",
            namespaced
        );
        apply_resource!(Fleet, "apps", "v1", "fleets", "fleet", namespaced);

        apply_validators!(Deployment, validators NameValidator);
        apply_validators!(ReplicaSet, validators NameValidator);
        apply_validators!(Fleet, validators NameValidator);
    }
}

pub mod authorization {
    pub mod v1 {
        use crate::validators::{NameValidator, NamespaceProhibitedValidator};
        use crate::{apply_resource, apply_validators};

        include!(concat!(env!("OUT_DIR"), "/tugboat.authorization.v1.rs"));

        apply_resource!(
            ClusterRole,
            "authorization",
            "v1",
            "clusterroles",
            "clusterrole",
            cluster
        );
        apply_resource!(
            ClusterRoleBinding,
            "authorization",
            "v1",
            "clusterrolebindings",
            "clusterrolebinding",
            cluster
        );
        apply_resource!(Role, "authorization", "v1", "roles", "role", namespaced);
        apply_resource!(
            RoleBinding,
            "authorization",
            "v1",
            "rolebindings",
            "rolebinding",
            namespaced
        );

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
        use crate::{apply_resource, apply_validators};

        include!(concat!(env!("OUT_DIR"), "/tugboat.coordination.v1.rs"));

        apply_resource!(Lease, "coordination", "v1", "leases", "lease", namespaced);

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
