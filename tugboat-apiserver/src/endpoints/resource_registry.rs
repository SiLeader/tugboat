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
use crate::endpoints::v1_coordination;
use crate::endpoints::v1_core;
use tugboat_resources::StaticResource;
use tugboat_resources::manifests::apps::v1::{Deployment, Fleet, ReplicaSet};
use tugboat_resources::manifests::coordination::v1::Lease;
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, ConfigMap, Namespace, NetworkClass, Node, PersistentVolume,
    PersistentVolumeClaim, Secret, Ship, ShipClass, StorageClass,
};
use utoipa_actix_web::service_config::ServiceConfig;

#[derive(Clone, Copy)]
pub(crate) struct ResourceOperations {
    pub(crate) create: bool,
    pub(crate) list: bool,
    pub(crate) read: bool,
    pub(crate) patch: bool,
    pub(crate) update: bool,
    pub(crate) delete: bool,
    pub(crate) status_patch: bool,
    pub(crate) status_update: bool,
}

impl ResourceOperations {
    pub(crate) fn has_status_subresource(self) -> bool {
        self.status_patch || self.status_update
    }

    pub(crate) fn resource_verbs(self) -> Vec<&'static str> {
        let mut verbs = Vec::new();
        if self.create {
            verbs.push("create");
        }
        if self.delete {
            verbs.push("delete");
        }
        if self.read {
            verbs.push("get");
        }
        if self.list {
            verbs.push("list");
        }
        if self.patch {
            verbs.push("patch");
        }
        if self.update {
            verbs.push("update");
        }
        if self.list {
            verbs.push("watch");
        }
        verbs
    }

    pub(crate) fn status_verbs(self) -> Vec<&'static str> {
        let mut verbs = Vec::new();
        if self.status_patch {
            verbs.push("patch");
        }
        if self.status_update {
            verbs.push("update");
        }
        verbs
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ResourceApiDescriptor {
    pub(crate) group: &'static str,
    pub(crate) version: &'static str,
    pub(crate) plural: &'static str,
    pub(crate) singular: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) namespaced: bool,
    pub(crate) operations: ResourceOperations,
    register: fn(&mut ServiceConfig),
}

impl ResourceApiDescriptor {
    fn new<T: StaticResource>(
        operations: ResourceOperations,
        register: fn(&mut ServiceConfig),
    ) -> Self {
        Self {
            group: T::group(),
            version: T::version(),
            plural: T::plural(),
            singular: T::singular(),
            kind: T::kind(),
            namespaced: !T::is_cluster_scoped(),
            operations,
            register,
        }
    }

    fn register(self, service: &mut ServiceConfig) {
        (self.register)(service);
    }
}

const CLUSTER_DEFAULT_OPS: ResourceOperations = ResourceOperations {
    create: true,
    list: true,
    read: true,
    patch: false,
    update: false,
    delete: false,
    status_patch: false,
    status_update: false,
};

const CLUSTER_STATUS_OPS: ResourceOperations = ResourceOperations {
    status_patch: true,
    status_update: true,
    ..CLUSTER_DEFAULT_OPS
};

const NAMESPACED_DEFAULT_OPS: ResourceOperations = ResourceOperations {
    create: true,
    list: true,
    read: true,
    patch: false,
    update: false,
    delete: false,
    status_patch: false,
    status_update: false,
};

const NAMESPACED_STATUS_OPS: ResourceOperations = ResourceOperations {
    status_patch: true,
    status_update: true,
    ..NAMESPACED_DEFAULT_OPS
};

const NODE_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    status_patch: true,
    status_update: true,
    ..CLUSTER_DEFAULT_OPS
};

const PERSISTENT_VOLUME_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    status_patch: true,
    status_update: true,
    ..CLUSTER_DEFAULT_OPS
};

const PERSISTENT_VOLUME_CLAIM_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    status_patch: true,
    status_update: true,
    ..NAMESPACED_DEFAULT_OPS
};

const SECRET_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    ..NAMESPACED_DEFAULT_OPS
};

const CONFIGMAP_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    ..NAMESPACED_DEFAULT_OPS
};

const STORAGE_CLASS_OPS: ResourceOperations = ResourceOperations {
    delete: true,
    ..CLUSTER_DEFAULT_OPS
};

const SHIP_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    status_patch: true,
    status_update: true,
    ..NAMESPACED_DEFAULT_OPS
};

const LEASE_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    ..NAMESPACED_DEFAULT_OPS
};

const WORKLOAD_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    ..NAMESPACED_DEFAULT_OPS
};

pub(crate) fn all_resource_apis() -> Vec<ResourceApiDescriptor> {
    vec![
        ResourceApiDescriptor::new::<Deployment>(WORKLOAD_OPS, v1_apps::register_deployment),
        ResourceApiDescriptor::new::<Fleet>(WORKLOAD_OPS, v1_apps::register_fleet),
        ResourceApiDescriptor::new::<ClusterNetworkClass>(
            CLUSTER_STATUS_OPS,
            v1_core::register_clusternetworkclass,
        ),
        ResourceApiDescriptor::new::<ConfigMap>(CONFIGMAP_OPS, v1_core::register_configmap),
        ResourceApiDescriptor::new::<Namespace>(CLUSTER_DEFAULT_OPS, v1_core::register_namespace),
        ResourceApiDescriptor::new::<Node>(NODE_OPS, v1_core::register_node),
        ResourceApiDescriptor::new::<PersistentVolume>(
            PERSISTENT_VOLUME_OPS,
            v1_core::register_persistent_volume,
        ),
        ResourceApiDescriptor::new::<NetworkClass>(
            NAMESPACED_STATUS_OPS,
            v1_core::register_networkclass,
        ),
        ResourceApiDescriptor::new::<PersistentVolumeClaim>(
            PERSISTENT_VOLUME_CLAIM_OPS,
            v1_core::register_persistent_volume_claim,
        ),
        ResourceApiDescriptor::new::<ReplicaSet>(WORKLOAD_OPS, v1_apps::register_replicaset),
        ResourceApiDescriptor::new::<Secret>(SECRET_OPS, v1_core::register_secret),
        ResourceApiDescriptor::new::<Ship>(SHIP_OPS, v1_core::register_ship),
        ResourceApiDescriptor::new::<ShipClass>(CLUSTER_DEFAULT_OPS, v1_core::register_shipclass),
        ResourceApiDescriptor::new::<StorageClass>(
            STORAGE_CLASS_OPS,
            v1_core::register_storage_class,
        ),
        ResourceApiDescriptor::new::<Lease>(LEASE_OPS, v1_coordination::register_lease),
    ]
}

pub(crate) fn resources_for(group: &str, version: &str) -> Vec<ResourceApiDescriptor> {
    all_resource_apis()
        .into_iter()
        .filter(|resource| resource.group == group && resource.version == version)
        .collect()
}

pub(crate) fn register_resource_apis(service: &mut ServiceConfig) {
    for resource in all_resource_apis() {
        resource.register(service);
    }
}
