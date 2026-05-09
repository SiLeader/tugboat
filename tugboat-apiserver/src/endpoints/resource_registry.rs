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

use crate::endpoints::v1_apps;
use crate::endpoints::v1_authorization;
use crate::endpoints::v1_coordination;
use crate::endpoints::v1_core;
use std::ops::Deref;
use tugboat_resources::StaticResource;
use tugboat_resources::manifests::apps::v1::{Deployment, Fleet, ReplicaSet};
use tugboat_resources::manifests::authorization::v1::{
    ClusterRole, ClusterRoleBinding, Role, RoleBinding,
};
use tugboat_resources::manifests::coordination::v1::Lease;
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, ConfigMap, Namespace, NetworkClass, Node, PersistentVolume,
    PersistentVolumeClaim, RuntimeClass, Secret, ServiceAccount, Ship, ShipClass, StorageClass,
};
use tugboat_resources::resource_api;
use tugboat_resources::resource_api::ResourceApiDescriptor as ResourceMetadataDescriptor;
use utoipa_actix_web::service_config::ServiceConfig;

#[derive(Clone, Copy)]
pub(crate) struct ResourceApiDescriptor {
    pub(crate) metadata: &'static ResourceMetadataDescriptor,
    register: fn(&mut ServiceConfig),
}

impl ResourceApiDescriptor {
    fn new<T: StaticResource>(
        metadata: &'static ResourceMetadataDescriptor,
        register: fn(&mut ServiceConfig),
    ) -> Self {
        debug_assert_eq!(
            T::descriptor(),
            metadata,
            "registered resource metadata differs from StaticResource metadata"
        );
        Self { metadata, register }
    }

    fn register(self, service: &mut ServiceConfig) {
        (self.register)(service);
    }
}

impl Deref for ResourceApiDescriptor {
    type Target = ResourceMetadataDescriptor;

    fn deref(&self) -> &Self::Target {
        self.metadata
    }
}

use std::sync::OnceLock;

static ALL_RESOURCE_APIS: OnceLock<Vec<ResourceApiDescriptor>> = OnceLock::new();

pub(crate) fn all_resource_apis() -> &'static [ResourceApiDescriptor] {
    ALL_RESOURCE_APIS.get_or_init(|| {
        vec![
            ResourceApiDescriptor::new::<ClusterRole>(
                &resource_api::CLUSTER_ROLE,
                v1_authorization::register_cluster_role,
            ),
            ResourceApiDescriptor::new::<ClusterRoleBinding>(
                &resource_api::CLUSTER_ROLE_BINDING,
                v1_authorization::register_cluster_role_binding,
            ),
            ResourceApiDescriptor::new::<Deployment>(
                &resource_api::DEPLOYMENT,
                v1_apps::register_deployment,
            ),
            ResourceApiDescriptor::new::<Fleet>(&resource_api::FLEET, v1_apps::register_fleet),
            ResourceApiDescriptor::new::<ClusterNetworkClass>(
                &resource_api::CLUSTER_NETWORK_CLASS,
                v1_core::register_clusternetworkclass,
            ),
            ResourceApiDescriptor::new::<ConfigMap>(
                &resource_api::CONFIG_MAP,
                v1_core::register_configmap,
            ),
            ResourceApiDescriptor::new::<Namespace>(
                &resource_api::NAMESPACE,
                v1_core::register_namespace,
            ),
            ResourceApiDescriptor::new::<Node>(&resource_api::NODE, v1_core::register_node),
            ResourceApiDescriptor::new::<PersistentVolume>(
                &resource_api::PERSISTENT_VOLUME,
                v1_core::register_persistent_volume,
            ),
            ResourceApiDescriptor::new::<NetworkClass>(
                &resource_api::NETWORK_CLASS,
                v1_core::register_networkclass,
            ),
            ResourceApiDescriptor::new::<PersistentVolumeClaim>(
                &resource_api::PERSISTENT_VOLUME_CLAIM,
                v1_core::register_persistent_volume_claim,
            ),
            ResourceApiDescriptor::new::<ReplicaSet>(
                &resource_api::REPLICA_SET,
                v1_apps::register_replicaset,
            ),
            ResourceApiDescriptor::new::<Role>(&resource_api::ROLE, v1_authorization::register_role),
            ResourceApiDescriptor::new::<RoleBinding>(
                &resource_api::ROLE_BINDING,
                v1_authorization::register_role_binding,
            ),
            ResourceApiDescriptor::new::<RuntimeClass>(
                &resource_api::RUNTIME_CLASS,
                v1_core::register_runtimeclass,
            ),
            ResourceApiDescriptor::new::<Secret>(&resource_api::SECRET, v1_core::register_secret),
            ResourceApiDescriptor::new::<ServiceAccount>(
                &resource_api::SERVICE_ACCOUNT,
                v1_core::register_service_account,
            ),
            ResourceApiDescriptor::new::<Ship>(&resource_api::SHIP, v1_core::register_ship),
            ResourceApiDescriptor::new::<ShipClass>(
                &resource_api::SHIP_CLASS,
                v1_core::register_shipclass,
            ),
            ResourceApiDescriptor::new::<StorageClass>(
                &resource_api::STORAGE_CLASS,
                v1_core::register_storage_class,
            ),
            ResourceApiDescriptor::new::<Lease>(&resource_api::LEASE, v1_coordination::register_lease),
        ]
    })
}

pub(crate) fn resources_for(group: &str, version: &str) -> Vec<ResourceApiDescriptor> {
    all_resource_apis()
        .iter()
        .filter(|resource| resource.group == group && resource.version == version)
        .copied()
        .collect()
}

pub(crate) fn register_resource_apis(service: &mut ServiceConfig) {
    for resource in all_resource_apis() {
        resource.register(service);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apiserver_registry_covers_resource_metadata_table() {
        let registered = all_resource_apis()
            .iter()
            .map(|descriptor| *descriptor.metadata)
            .collect::<Vec<_>>();

        assert_eq!(
            registered.as_slice(),
            resource_api::all_resource_descriptors()
        );
    }
}
