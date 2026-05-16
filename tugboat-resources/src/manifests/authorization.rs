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
