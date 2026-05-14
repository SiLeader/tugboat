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

mod protobuf;

use crate::error::Error;
use crate::serializer::protobuf::ProtobufSerializer;
use tugboat_resources::manifests::apps::v1::{Deployment, Fleet, ReplicaSet};
use tugboat_resources::manifests::authorization::v1::{
    ClusterRole, ClusterRoleBinding, Role, RoleBinding,
};
use tugboat_resources::manifests::coordination::v1::Lease;
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, ConfigMap, Namespace, NetworkClass, Node, PersistentVolume,
    PersistentVolumeClaim, RuntimeClass, Secret, ServiceAccount, Ship, ShipClass, StorageClass,
};
use tugboat_resources::manifests::meta::v1::TypeMeta;
use tugboat_resources::manifests::snapshot::v1::{
    VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotContent,
};
use tugboat_resources::{Resource, StaticResource};

pub trait Serializable: Resource + Sized {
    fn serialize(&self) -> Result<Vec<u8>, Error>;
    fn deserialize(data: &[u8]) -> Result<Self, Error>;
}

pub trait StaticSerializable: Serializable + StaticResource {}

pub trait Serializer {
    fn is_supported(&self, type_meta: &TypeMeta) -> bool;

    fn serialize_protobuf<T: prost::Message>(
        &self,
        _type_meta: &TypeMeta,
        _value: &T,
    ) -> Result<Vec<u8>, Error> {
        Err(Error::UnsupportedType)
    }

    fn deserialize_protobuf<T>(&self, _type_meta: &TypeMeta, _data: &[u8]) -> Result<T, Error>
    where
        T: prost::Message + Default,
    {
        Err(Error::UnsupportedType)
    }
}

macro_rules! protobuf_serializable {
    ($ty:ident) => {
        impl Serializable for $ty {
            fn serialize(&self) -> Result<Vec<u8>, Error> {
                ProtobufSerializer.serialize_protobuf(&Self::type_meta(), self)
            }

            fn deserialize(data: &[u8]) -> Result<Self, Error> {
                ProtobufSerializer.deserialize_protobuf(&Self::type_meta(), data)
            }
        }

        impl StaticSerializable for $ty {}
    };
}

protobuf_serializable!(ClusterNetworkClass);
protobuf_serializable!(ClusterRole);
protobuf_serializable!(ClusterRoleBinding);
protobuf_serializable!(ConfigMap);
protobuf_serializable!(Deployment);
protobuf_serializable!(Fleet);
protobuf_serializable!(Lease);
protobuf_serializable!(Namespace);
protobuf_serializable!(NetworkClass);
protobuf_serializable!(Node);
protobuf_serializable!(Role);
protobuf_serializable!(RoleBinding);
protobuf_serializable!(RuntimeClass);
protobuf_serializable!(ServiceAccount);
protobuf_serializable!(Ship);
protobuf_serializable!(ShipClass);
protobuf_serializable!(StorageClass);
protobuf_serializable!(Secret);
protobuf_serializable!(PersistentVolume);
protobuf_serializable!(PersistentVolumeClaim);
protobuf_serializable!(ReplicaSet);
protobuf_serializable!(VolumeSnapshot);
protobuf_serializable!(VolumeSnapshotContent);
protobuf_serializable!(VolumeSnapshotClass);

#[cfg(test)]
mod tests {
    use super::Serializable;
    use super::StaticSerializable;
    use std::collections::BTreeSet;
    use std::collections::HashMap;
    use tugboat_resources::manifests::apps::v1::{Deployment, Fleet, ReplicaSet};
    use tugboat_resources::manifests::authorization::v1::{
        ClusterRole, ClusterRoleBinding, Role, RoleBinding,
    };
    use tugboat_resources::manifests::coordination::v1::Lease;
    use tugboat_resources::manifests::core::v1::{
        ClusterNetworkClass, ConfigMap, Namespace, NetworkClass, Node, NodeSelector,
        NodeSelectorRequirement, NodeSelectorTerm, PersistentVolume, PersistentVolumeClaim,
        PersistentVolumeSpec, RuntimeClass, Secret, ServiceAccount, Ship, ShipClass, StorageClass,
        StorageClassSpec, TopologySelectorLabelRequirement, TopologySelectorTerm,
        VolumeNodeAffinity,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;
    use tugboat_resources::manifests::snapshot::v1::{
        VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotClassSpec, VolumeSnapshotContent,
        VolumeSnapshotContentSource, VolumeSnapshotContentSpec, VolumeSnapshotSource,
        VolumeSnapshotSpec,
    };
    use tugboat_resources::resource_api;

    fn serializable_descriptor<T: StaticSerializable>() -> resource_api::ResourceApiDescriptor {
        *T::descriptor()
    }

    fn registered_serializable_descriptors() -> Vec<resource_api::ResourceApiDescriptor> {
        vec![
            serializable_descriptor::<ClusterNetworkClass>(),
            serializable_descriptor::<ClusterRole>(),
            serializable_descriptor::<ClusterRoleBinding>(),
            serializable_descriptor::<ConfigMap>(),
            serializable_descriptor::<Deployment>(),
            serializable_descriptor::<Fleet>(),
            serializable_descriptor::<Lease>(),
            serializable_descriptor::<Namespace>(),
            serializable_descriptor::<NetworkClass>(),
            serializable_descriptor::<Node>(),
            serializable_descriptor::<Role>(),
            serializable_descriptor::<RoleBinding>(),
            serializable_descriptor::<RuntimeClass>(),
            serializable_descriptor::<ServiceAccount>(),
            serializable_descriptor::<Ship>(),
            serializable_descriptor::<ShipClass>(),
            serializable_descriptor::<StorageClass>(),
            serializable_descriptor::<Secret>(),
            serializable_descriptor::<PersistentVolume>(),
            serializable_descriptor::<PersistentVolumeClaim>(),
            serializable_descriptor::<ReplicaSet>(),
            serializable_descriptor::<VolumeSnapshot>(),
            serializable_descriptor::<VolumeSnapshotContent>(),
            serializable_descriptor::<VolumeSnapshotClass>(),
        ]
    }

    #[test]
    fn serializer_registration_covers_every_resource_descriptor() {
        let serializable = registered_serializable_descriptors()
            .into_iter()
            .map(|descriptor| (descriptor.group, descriptor.version, descriptor.plural))
            .collect::<BTreeSet<_>>();
        let expected = resource_api::all_resource_descriptors()
            .iter()
            .map(|descriptor| (descriptor.group, descriptor.version, descriptor.plural))
            .collect::<BTreeSet<_>>();

        assert_eq!(serializable, expected);
    }

    #[test]
    fn can_round_trip_configmap() {
        let config_map = ConfigMap {
            object_meta: Some(ObjectMeta {
                name: Some("settings".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            data: HashMap::from([("key".to_string(), "value".to_string())]),
            ..Default::default()
        };

        let encoded = config_map.serialize().unwrap();
        let decoded = ConfigMap::deserialize(&encoded).unwrap();

        assert_eq!(
            decoded.object_meta.unwrap().name.as_deref(),
            Some("settings")
        );
        assert_eq!(decoded.data.get("key").map(String::as_str), Some("value"));
    }

    #[test]
    fn can_round_trip_persistent_volume_node_affinity() {
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

        let encoded = pv.serialize().unwrap();
        let decoded = PersistentVolume::deserialize(&encoded).unwrap();
        let requirement = decoded
            .spec
            .as_ref()
            .and_then(|spec| spec.node_affinity.as_ref())
            .and_then(|affinity| affinity.required.as_ref())
            .and_then(|selector| selector.node_selector_terms.first())
            .and_then(|term| term.match_expressions.first())
            .expect("node affinity requirement should round-trip");

        assert_eq!(requirement.key, "topology.tugboat.cloud/zone");
        assert_eq!(requirement.operator, "In");
        assert_eq!(requirement.values, vec!["us-east-a".to_string()]);
    }

    #[test]
    fn can_round_trip_storage_class_topology_fields() {
        let storage_class = StorageClass {
            object_meta: Some(ObjectMeta {
                name: Some("fast".to_string()),
                ..Default::default()
            }),
            spec: Some(StorageClassSpec {
                provisioner: "example.csi.driver".to_string(),
                volume_binding_mode: Some("WaitForFirstConsumer".to_string()),
                allowed_topologies: vec![TopologySelectorTerm {
                    match_label_expressions: vec![TopologySelectorLabelRequirement {
                        key: "topology.tugboat.cloud/zone".to_string(),
                        values: vec!["us-east-a".to_string()],
                    }],
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let encoded = storage_class.serialize().unwrap();
        let decoded = StorageClass::deserialize(&encoded).unwrap();
        let spec = decoded.spec.expect("spec should round-trip");

        assert_eq!(
            spec.volume_binding_mode.as_deref(),
            Some("WaitForFirstConsumer")
        );
        assert_eq!(spec.allowed_topologies.len(), 1);
        assert_eq!(
            spec.allowed_topologies[0].match_label_expressions[0].key,
            "topology.tugboat.cloud/zone"
        );
    }

    #[test]
    fn can_round_trip_snapshot_resources() {
        let snapshot = VolumeSnapshot {
            object_meta: Some(ObjectMeta {
                name: Some("snap-a".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(VolumeSnapshotSpec {
                source: Some(VolumeSnapshotSource {
                    persistent_volume_claim_name: Some("data".to_string()),
                    ..Default::default()
                }),
                volume_snapshot_class_name: Some("fast".to_string()),
            }),
            ..Default::default()
        };
        let decoded_snapshot = VolumeSnapshot::deserialize(&snapshot.serialize().unwrap()).unwrap();
        assert_eq!(
            decoded_snapshot
                .spec
                .as_ref()
                .and_then(|spec| spec.source.as_ref())
                .and_then(|source| source.persistent_volume_claim_name.as_deref()),
            Some("data")
        );

        let content = VolumeSnapshotContent {
            object_meta: Some(ObjectMeta {
                name: Some("content-a".to_string()),
                ..Default::default()
            }),
            spec: Some(VolumeSnapshotContentSpec {
                deletion_policy: "Retain".to_string(),
                source: Some(VolumeSnapshotContentSource {
                    snapshot_handle: Some("snap-handle".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let decoded_content =
            VolumeSnapshotContent::deserialize(&content.serialize().unwrap()).unwrap();
        assert_eq!(
            decoded_content
                .spec
                .as_ref()
                .and_then(|spec| spec.source.as_ref())
                .and_then(|source| source.snapshot_handle.as_deref()),
            Some("snap-handle")
        );

        let class = VolumeSnapshotClass {
            object_meta: Some(ObjectMeta {
                name: Some("fast".to_string()),
                ..Default::default()
            }),
            spec: Some(VolumeSnapshotClassSpec {
                driver: "csi.tugboat.cloud/fast".to_string(),
                deletion_policy: "Delete".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let decoded_class = VolumeSnapshotClass::deserialize(&class.serialize().unwrap()).unwrap();
        assert_eq!(
            decoded_class
                .spec
                .as_ref()
                .map(|spec| spec.deletion_policy.as_str()),
            Some("Delete")
        );
    }
}
