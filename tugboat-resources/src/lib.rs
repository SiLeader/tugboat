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

use crate::manifests::meta::v1::{ObjectMeta, Time, TypeMeta};

pub mod manifests;
pub mod resource_api;
pub mod resource_version;
pub mod sized;
pub mod validators;

pub const NODE_ARCH_LABEL_KEY: &str = "tugboat.cloud/arch";
pub const NODE_RUNTIME_CLASS_LABEL_KEY: &str = "tugboat.cloud/runtime-class";
pub const NODE_REGION_LABEL_KEY: &str = "topology.tugboat.cloud/region";
pub const NODE_ZONE_LABEL_KEY: &str = "topology.tugboat.cloud/zone";
pub const NODE_HOSTNAME_LABEL_KEY: &str = "topology.tugboat.cloud/host";
pub const SERVICE_ACCOUNT_NAME_ANNOTATION: &str = "tugboat.cloud/service-account.name";
pub const SERVICE_ACCOUNT_TOKEN_SECRET_TYPE: &str = "tugboat.cloud/service-account-token";
pub const SELECTED_NODE_ANNOTATION: &str = "volume.tugboat.cloud/selected-node";

pub trait Resource {
    fn type_meta() -> TypeMeta;
}

pub trait SetTypeMeta {
    fn set_type_meta(&mut self, type_meta: TypeMeta);
}

pub trait StaticResource: Resource {
    fn descriptor() -> &'static resource_api::ResourceApiDescriptor;

    fn group() -> &'static str {
        Self::descriptor().group
    }

    fn version() -> &'static str {
        Self::descriptor().version
    }

    fn kind() -> &'static str {
        Self::descriptor().kind
    }

    fn plural() -> &'static str {
        Self::descriptor().plural
    }

    fn singular() -> &'static str {
        Self::descriptor().singular
    }

    fn is_cluster_scoped() -> bool {
        Self::descriptor().cluster_scoped()
    }
}

pub trait ClusterScopedResource: StaticResource {}

pub trait NamespacedResource: StaticResource {}

pub trait ObjectMetaResource: Resource {
    fn object_meta(&self) -> &Option<ObjectMeta>;
    fn object_meta_mut(&mut self) -> &mut Option<ObjectMeta>;

    fn name(&self) -> Option<&str> {
        self.object_meta().as_ref()?.name.as_deref()
    }

    fn namespace(&self) -> Option<&str> {
        self.object_meta().as_ref()?.namespace.as_deref()
    }

    fn finalizers(&self) -> &[String] {
        self.object_meta()
            .as_ref()
            .map(|meta| meta.finalizers.as_slice())
            .unwrap_or_default()
    }

    fn has_finalizers(&self) -> bool {
        !self.finalizers().is_empty()
    }

    fn has_finalizer(&self, finalizer_name: &str) -> bool {
        self.finalizers().iter().any(|item| item == finalizer_name)
    }

    fn add_finalizer(&mut self, finalizer_name: impl Into<String>) -> bool {
        let finalizer_name = finalizer_name.into();
        let meta = self
            .object_meta_mut()
            .get_or_insert_with(ObjectMeta::default);
        if meta.finalizers.iter().any(|item| item == &finalizer_name) {
            return false;
        }
        meta.finalizers.push(finalizer_name);
        true
    }

    fn remove_finalizer(&mut self, finalizer_name: &str) -> bool {
        let Some(meta) = self.object_meta_mut().as_mut() else {
            return false;
        };
        let original_len = meta.finalizers.len();
        meta.finalizers.retain(|item| item != finalizer_name);
        original_len != meta.finalizers.len()
    }

    fn deletion_timestamp(&self) -> Option<&Time> {
        self.object_meta()
            .as_ref()
            .and_then(|meta| meta.deletion_timestamp.as_ref())
    }

    fn mark_for_deletion(&mut self, timestamp: Time) -> bool {
        let meta = self
            .object_meta_mut()
            .get_or_insert_with(ObjectMeta::default);
        if meta.deletion_timestamp.is_some() {
            return false;
        }
        meta.deletion_timestamp = Some(timestamp);
        true
    }

    fn modify_object_meta(&mut self, f: impl FnOnce(&mut Option<ObjectMeta>)) {
        f(self.object_meta_mut());
    }
    fn set_object_meta(&mut self, object_meta: Option<ObjectMeta>) {
        *self.object_meta_mut() = object_meta;
    }
}

pub trait ShipMigrationExt {
    fn has_active_migration(&self) -> bool;
}

impl ShipMigrationExt for crate::manifests::core::v1::Ship {
    fn has_active_migration(&self) -> bool {
        matches!(
            self.status
                .as_ref()
                .and_then(|status| status.migration.as_ref())
                .map(|migration| migration.phase.as_str()),
            Some("Pending" | "Ready" | "Migrating")
        )
    }
}

impl<T: StaticResource> Resource for T {
    fn type_meta() -> TypeMeta {
        TypeMeta {
            api_version: Some(if Self::group() == "core" || Self::group().is_empty() {
                Self::version().to_string()
            } else {
                format!("{}/{}", Self::group(), Self::version())
            }),
            kind: Some(Self::kind().to_string()),
        }
    }
}

#[macro_export]
macro_rules! apply_resource {
    ($ty:ident, $descriptor:path, $cluster_scoped:literal) => {
        impl $crate::StaticResource for $ty {
            fn descriptor() -> &'static $crate::resource_api::ResourceApiDescriptor {
                &$descriptor
            }
        }

        impl $crate::ObjectMetaResource for $ty {
            fn object_meta(&self) -> &Option<$crate::manifests::meta::v1::ObjectMeta> {
                &self.object_meta
            }

            fn object_meta_mut(&mut self) -> &mut Option<$crate::manifests::meta::v1::ObjectMeta> {
                &mut self.object_meta
            }
        }

        impl $crate::SetTypeMeta for $ty {
            fn set_type_meta(&mut self, type_meta: $crate::manifests::meta::v1::TypeMeta) {
                self.type_meta = Some(type_meta);
            }
        }
    };

    ($ty:ident, $descriptor:path, namespaced) => {
        apply_resource!($ty, $descriptor, false);

        impl $crate::NamespacedResource for $ty {}
    };

    ($ty:ident, $descriptor:path, cluster) => {
        apply_resource!($ty, $descriptor, true);

        impl $crate::ClusterScopedResource for $ty {}
    };
}

#[cfg(test)]
mod tests {
    use crate::StaticResource;
    use crate::manifests::apiextensions::v1::CustomResourceDefinition;
    use crate::manifests::apps::v1::{Deployment, Fleet, ReplicaSet};
    use crate::manifests::authorization::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
    use crate::manifests::coordination::v1::Lease;
    use crate::manifests::core::v1::{
        ClusterNetworkClass, ConfigMap, Namespace, NetworkClass, Node, PersistentVolume,
        PersistentVolumeClaim, RuntimeClass, Secret, ServiceAccount, ShipClass, StorageClass,
    };
    use crate::manifests::core::v1::{Ship, ShipMigrationStatus, ShipSpec, ShipStatus};
    use crate::manifests::meta::v1::{ObjectMeta, Time};
    use crate::manifests::snapshot::v1::{
        VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotContent,
    };
    use crate::{ObjectMetaResource, ShipMigrationExt, resource_api};
    use serde_json::json;

    macro_rules! assert_static_descriptor {
        ($ty:ty, $descriptor:expr) => {{
            let descriptor = <$ty as StaticResource>::descriptor();
            assert_eq!(descriptor, &$descriptor);
            assert_eq!(<$ty as StaticResource>::group(), $descriptor.group);
            assert_eq!(<$ty as StaticResource>::version(), $descriptor.version);
            assert_eq!(<$ty as StaticResource>::kind(), $descriptor.kind);
            assert_eq!(<$ty as StaticResource>::plural(), $descriptor.plural);
            assert_eq!(<$ty as StaticResource>::singular(), $descriptor.singular);
            assert_eq!(
                <$ty as StaticResource>::is_cluster_scoped(),
                $descriptor.cluster_scoped()
            );
        }};
    }

    #[test]
    fn static_resource_metadata_comes_from_resource_descriptors() {
        assert_static_descriptor!(
            CustomResourceDefinition,
            resource_api::CUSTOM_RESOURCE_DEFINITION
        );
        assert_static_descriptor!(ClusterRole, resource_api::CLUSTER_ROLE);
        assert_static_descriptor!(ClusterRoleBinding, resource_api::CLUSTER_ROLE_BINDING);
        assert_static_descriptor!(Deployment, resource_api::DEPLOYMENT);
        assert_static_descriptor!(Fleet, resource_api::FLEET);
        assert_static_descriptor!(ClusterNetworkClass, resource_api::CLUSTER_NETWORK_CLASS);
        assert_static_descriptor!(ConfigMap, resource_api::CONFIG_MAP);
        assert_static_descriptor!(Namespace, resource_api::NAMESPACE);
        assert_static_descriptor!(Node, resource_api::NODE);
        assert_static_descriptor!(PersistentVolume, resource_api::PERSISTENT_VOLUME);
        assert_static_descriptor!(NetworkClass, resource_api::NETWORK_CLASS);
        assert_static_descriptor!(PersistentVolumeClaim, resource_api::PERSISTENT_VOLUME_CLAIM);
        assert_static_descriptor!(ReplicaSet, resource_api::REPLICA_SET);
        assert_static_descriptor!(Role, resource_api::ROLE);
        assert_static_descriptor!(RoleBinding, resource_api::ROLE_BINDING);
        assert_static_descriptor!(RuntimeClass, resource_api::RUNTIME_CLASS);
        assert_static_descriptor!(Secret, resource_api::SECRET);
        assert_static_descriptor!(ServiceAccount, resource_api::SERVICE_ACCOUNT);
        assert_static_descriptor!(Ship, resource_api::SHIP);
        assert_static_descriptor!(ShipClass, resource_api::SHIP_CLASS);
        assert_static_descriptor!(StorageClass, resource_api::STORAGE_CLASS);
        assert_static_descriptor!(Lease, resource_api::LEASE);
        assert_static_descriptor!(VolumeSnapshot, resource_api::VOLUME_SNAPSHOT);
        assert_static_descriptor!(VolumeSnapshotContent, resource_api::VOLUME_SNAPSHOT_CONTENT);
        assert_static_descriptor!(VolumeSnapshotClass, resource_api::VOLUME_SNAPSHOT_CLASS);
    }

    #[test]
    fn can_manage_finalizers() {
        let mut ship = Ship {
            object_meta: Some(ObjectMeta::default()),
            ..Default::default()
        };

        assert!(ship.add_finalizer("example.com/finalizer"));
        assert!(ship.has_finalizer("example.com/finalizer"));
        assert!(!ship.add_finalizer("example.com/finalizer"));
        assert!(ship.remove_finalizer("example.com/finalizer"));
        assert!(!ship.has_finalizers());
    }

    #[test]
    fn can_mark_resource_for_deletion() {
        let mut ship = Ship::default();
        let timestamp = Time::now();

        assert!(ship.mark_for_deletion(timestamp));
        assert!(ship.deletion_timestamp().is_some());
        assert!(!ship.mark_for_deletion(timestamp));
    }

    #[test]
    fn ship_serializes_migration_fields_in_camel_case() {
        let ship = Ship {
            spec: Some(ShipSpec {
                target_node_name: Some("node-b".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: "Ready".to_string(),
                    target_address: Some("10.0.0.8".to_string()),
                    target_port: Some(4444),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let json = serde_json::to_value(ship).unwrap();

        assert_eq!(json["spec"]["targetNodeName"], json!("node-b"));
        assert_eq!(json["status"]["migration"]["phase"], json!("Ready"));
        assert_eq!(
            json["status"]["migration"]["targetAddress"],
            json!("10.0.0.8")
        );
        assert_eq!(json["status"]["migration"]["targetPort"], json!(4444));
    }

    #[test]
    fn ship_has_active_migration_only_for_active_phases() {
        for phase in ["Pending", "Ready", "Migrating"] {
            let ship = Ship {
                status: Some(ShipStatus {
                    migration: Some(ShipMigrationStatus {
                        phase: phase.to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };

            assert!(ship.has_active_migration(), "phase={phase}");
        }

        let ship = Ship {
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: "Completed".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(!ship.has_active_migration());
    }
}
