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

use crate::auth::constants::{
    CLUSTER_ROLE_KIND, GROUP_SUBJECT_KIND, RBAC_API_GROUP, SYSTEM_MASTERS_GROUP,
};
use tugboat_resource_store::ResourceStore;
use tugboat_resource_store::error::Error;
use tugboat_resources::Resource;
use tugboat_resources::manifests::authorization::v1::{
    ClusterRole, ClusterRoleBinding, PolicyRule, RoleRef, Subject,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, Time};
use uuid::Uuid;
const CLUSTER_ADMIN_ROLE: &str = "cluster-admin";
const ADMIN_ROLE: &str = "admin";
const EDIT_ROLE: &str = "edit";
const VIEW_ROLE: &str = "view";
const CLUSTER_ADMIN_BINDING: &str = "cluster-admin";

pub(crate) async fn bootstrap_default_rbac(store: &ResourceStore) -> Result<(), Error> {
    store
        .put_if_not_exists(cluster_role(
            CLUSTER_ADMIN_ROLE,
            vec![policy_rule(&["*"], &["*"], &["*"])],
        ))
        .await?;
    store
        .put_if_not_exists(cluster_role(
            ADMIN_ROLE,
            vec![
                policy_rule(&["core", "apps"], &["*"], &["*"]),
                policy_rule(&[RBAC_API_GROUP], &["roles", "rolebindings"], &["*"]),
            ],
        ))
        .await?;
    store
        .put_if_not_exists(cluster_role(
            EDIT_ROLE,
            vec![
                policy_rule(
                    &["core", "apps"],
                    &["*"],
                    &[
                        "create", "get", "list", "watch", "update", "patch", "delete",
                    ],
                ),
                policy_rule(
                    &[RBAC_API_GROUP],
                    &["roles", "rolebindings"],
                    &["get", "list", "watch"],
                ),
            ],
        ))
        .await?;
    store
        .put_if_not_exists(cluster_role(
            VIEW_ROLE,
            vec![policy_rule(
                &["core", "apps"],
                &["*"],
                &["get", "list", "watch"],
            )],
        ))
        .await?;
    store
        .put_if_not_exists(cluster_role_binding(
            CLUSTER_ADMIN_BINDING,
            CLUSTER_ADMIN_ROLE,
            vec![Subject {
                kind: GROUP_SUBJECT_KIND.to_string(),
                api_group: RBAC_API_GROUP.to_string(),
                name: SYSTEM_MASTERS_GROUP.to_string(),
                namespace: None,
            }],
        ))
        .await?;

    Ok(())
}

fn cluster_role(name: &str, rules: Vec<PolicyRule>) -> ClusterRole {
    ClusterRole {
        type_meta: Some(ClusterRole::type_meta()),
        object_meta: Some(cluster_object_meta(name)),
        rules,
    }
}

fn cluster_role_binding(name: &str, role_name: &str, subjects: Vec<Subject>) -> ClusterRoleBinding {
    ClusterRoleBinding {
        type_meta: Some(ClusterRoleBinding::type_meta()),
        object_meta: Some(cluster_object_meta(name)),
        subjects,
        role_ref: Some(RoleRef {
            api_group: RBAC_API_GROUP.to_string(),
            kind: CLUSTER_ROLE_KIND.to_string(),
            name: role_name.to_string(),
        }),
    }
}

fn cluster_object_meta(name: &str) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.to_string()),
        uid: Some(Uuid::new_v4().to_string()),
        creation_timestamp: Some(Time::now()),
        generation: Some(1),
        ..Default::default()
    }
}

fn policy_rule(api_groups: &[&str], resources: &[&str], verbs: &[&str]) -> PolicyRule {
    PolicyRule {
        api_groups: api_groups
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        resources: resources.iter().map(|value| (*value).to_string()).collect(),
        resource_names: Vec::new(),
        verbs: verbs.iter().map(|value| (*value).to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{cluster_role, cluster_role_binding, policy_rule};

    #[test]
    fn cluster_role_has_expected_metadata_and_rules() {
        let role = cluster_role("view", vec![policy_rule(&["core"], &["ships"], &["get"])]);

        assert_eq!(
            role.object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref()),
            Some("view")
        );
        assert_eq!(role.rules.len(), 1);
        assert_eq!(role.rules[0].verbs, vec!["get".to_string()]);
        assert!(role.type_meta.is_some());
    }

    #[test]
    fn cluster_role_binding_targets_cluster_role() {
        let binding = cluster_role_binding("cluster-admin", "cluster-admin", Vec::new());

        let role_ref = binding.role_ref.expect("role_ref should be set");
        assert_eq!(role_ref.kind, "ClusterRole");
        assert_eq!(role_ref.name, "cluster-admin");
        assert_eq!(
            binding
                .object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref()),
            Some("cluster-admin")
        );
    }
}
