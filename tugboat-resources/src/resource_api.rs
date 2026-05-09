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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceScope {
    Cluster,
    Namespaced,
}

impl ResourceScope {
    pub const fn is_cluster_scoped(self) -> bool {
        matches!(self, Self::Cluster)
    }

    pub const fn is_namespaced(self) -> bool {
        matches!(self, Self::Namespaced)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceOperations {
    pub create: bool,
    pub list: bool,
    pub read: bool,
    pub patch: bool,
    pub update: bool,
    pub delete: bool,
    pub status_patch: bool,
    pub status_update: bool,
}

impl ResourceOperations {
    pub const fn has_status_subresource(self) -> bool {
        self.status_patch || self.status_update
    }

    pub fn resource_verbs(self) -> Vec<&'static str> {
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

    pub fn status_verbs(self) -> Vec<&'static str> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceApiDescriptor {
    pub group: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub plural: &'static str,
    pub singular: &'static str,
    pub scope: ResourceScope,
    pub operations: ResourceOperations,
}

impl ResourceApiDescriptor {
    pub const fn namespaced(self) -> bool {
        self.scope.is_namespaced()
    }

    pub const fn cluster_scoped(self) -> bool {
        self.scope.is_cluster_scoped()
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

const SERVICE_ACCOUNT_OPS: ResourceOperations = ResourceOperations {
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
    status_patch: true,
    status_update: true,
    ..NAMESPACED_DEFAULT_OPS
};

const CLUSTER_RBAC_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    ..CLUSTER_DEFAULT_OPS
};

const NAMESPACED_RBAC_OPS: ResourceOperations = ResourceOperations {
    patch: true,
    update: true,
    delete: true,
    ..NAMESPACED_DEFAULT_OPS
};

macro_rules! descriptor {
    ($name:ident, $group:literal, $version:literal, $kind:literal, $plural:literal, $singular:literal, $scope:ident, $operations:expr) => {
        pub const $name: ResourceApiDescriptor = ResourceApiDescriptor {
            group: $group,
            version: $version,
            kind: $kind,
            plural: $plural,
            singular: $singular,
            scope: ResourceScope::$scope,
            operations: $operations,
        };
    };
}

descriptor!(
    CONFIG_MAP,
    "core",
    "v1",
    "ConfigMap",
    "configmaps",
    "configmap",
    Namespaced,
    CONFIGMAP_OPS
);
descriptor!(
    NAMESPACE,
    "core",
    "v1",
    "Namespace",
    "namespaces",
    "namespace",
    Cluster,
    CLUSTER_DEFAULT_OPS
);
descriptor!(
    NETWORK_CLASS,
    "core",
    "v1",
    "NetworkClass",
    "networkclasses",
    "networkclass",
    Namespaced,
    NAMESPACED_STATUS_OPS
);
descriptor!(
    CLUSTER_NETWORK_CLASS,
    "core",
    "v1",
    "ClusterNetworkClass",
    "clusternetworkclasses",
    "clusternetworkclass",
    Cluster,
    CLUSTER_STATUS_OPS
);
descriptor!(
    NODE, "core", "v1", "Node", "nodes", "node", Cluster, NODE_OPS
);
descriptor!(
    PERSISTENT_VOLUME,
    "core",
    "v1",
    "PersistentVolume",
    "persistentvolumes",
    "persistentvolume",
    Cluster,
    PERSISTENT_VOLUME_OPS
);
descriptor!(
    PERSISTENT_VOLUME_CLAIM,
    "core",
    "v1",
    "PersistentVolumeClaim",
    "persistentvolumeclaims",
    "persistentvolumeclaim",
    Namespaced,
    PERSISTENT_VOLUME_CLAIM_OPS
);
descriptor!(
    SECRET, "core", "v1", "Secret", "secrets", "secret", Namespaced, SECRET_OPS
);
descriptor!(
    SERVICE_ACCOUNT,
    "core",
    "v1",
    "ServiceAccount",
    "serviceaccounts",
    "serviceaccount",
    Namespaced,
    SERVICE_ACCOUNT_OPS
);
descriptor!(
    RUNTIME_CLASS,
    "core",
    "v1",
    "RuntimeClass",
    "runtimeclasses",
    "runtimeclass",
    Cluster,
    STORAGE_CLASS_OPS
);
descriptor!(
    STORAGE_CLASS,
    "core",
    "v1",
    "StorageClass",
    "storageclasses",
    "storageclass",
    Cluster,
    STORAGE_CLASS_OPS
);
descriptor!(
    SHIP, "core", "v1", "Ship", "ships", "ship", Namespaced, SHIP_OPS
);
descriptor!(
    SHIP_CLASS,
    "core",
    "v1",
    "ShipClass",
    "shipclasses",
    "shipclass",
    Cluster,
    CLUSTER_DEFAULT_OPS
);
descriptor!(
    DEPLOYMENT,
    "apps",
    "v1",
    "Deployment",
    "deployments",
    "deployment",
    Namespaced,
    WORKLOAD_OPS
);
descriptor!(
    REPLICA_SET,
    "apps",
    "v1",
    "ReplicaSet",
    "replicasets",
    "replicaset",
    Namespaced,
    WORKLOAD_OPS
);
descriptor!(
    FLEET,
    "apps",
    "v1",
    "Fleet",
    "fleets",
    "fleet",
    Namespaced,
    WORKLOAD_OPS
);
descriptor!(
    CLUSTER_ROLE,
    "authorization",
    "v1",
    "ClusterRole",
    "clusterroles",
    "clusterrole",
    Cluster,
    CLUSTER_RBAC_OPS
);
descriptor!(
    CLUSTER_ROLE_BINDING,
    "authorization",
    "v1",
    "ClusterRoleBinding",
    "clusterrolebindings",
    "clusterrolebinding",
    Cluster,
    CLUSTER_RBAC_OPS
);
descriptor!(
    ROLE,
    "authorization",
    "v1",
    "Role",
    "roles",
    "role",
    Namespaced,
    NAMESPACED_RBAC_OPS
);
descriptor!(
    ROLE_BINDING,
    "authorization",
    "v1",
    "RoleBinding",
    "rolebindings",
    "rolebinding",
    Namespaced,
    NAMESPACED_RBAC_OPS
);
descriptor!(
    LEASE,
    "coordination",
    "v1",
    "Lease",
    "leases",
    "lease",
    Namespaced,
    LEASE_OPS
);

pub const ALL_RESOURCE_DESCRIPTORS: &[ResourceApiDescriptor] = &[
    CLUSTER_ROLE,
    CLUSTER_ROLE_BINDING,
    DEPLOYMENT,
    FLEET,
    CLUSTER_NETWORK_CLASS,
    CONFIG_MAP,
    NAMESPACE,
    NODE,
    PERSISTENT_VOLUME,
    NETWORK_CLASS,
    PERSISTENT_VOLUME_CLAIM,
    REPLICA_SET,
    ROLE,
    ROLE_BINDING,
    RUNTIME_CLASS,
    SECRET,
    SERVICE_ACCOUNT,
    SHIP,
    SHIP_CLASS,
    STORAGE_CLASS,
    LEASE,
];

pub fn all_resource_descriptors() -> &'static [ResourceApiDescriptor] {
    ALL_RESOURCE_DESCRIPTORS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn resource_descriptors_have_unique_group_version_plural() {
        let mut seen = BTreeSet::new();

        for descriptor in all_resource_descriptors() {
            assert!(
                seen.insert((descriptor.group, descriptor.version, descriptor.plural)),
                "duplicate descriptor for {}/{}/{}",
                descriptor.group,
                descriptor.version,
                descriptor.plural
            );
        }
    }

    #[test]
    fn status_descriptors_expose_status_verbs_only_when_enabled() {
        for descriptor in all_resource_descriptors() {
            let status_verbs = descriptor.operations.status_verbs();
            if descriptor.operations.has_status_subresource() {
                assert_eq!(status_verbs, vec!["patch", "update"]);
            } else {
                assert!(status_verbs.is_empty(), "{descriptor:?}");
            }
        }
    }
}
