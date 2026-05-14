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
use std::collections::HashMap;
use tugboat_resource_store::ResourceStore;
use tugboat_resource_store::error::Error;
use tugboat_resources::Resource;
use tugboat_resources::manifests::authorization::v1::{
    AggregationRule, ClusterRole, ClusterRoleBinding, LabelSelector, PolicyRule, RoleRef, Subject,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, Time};
use uuid::Uuid;

const CLUSTER_ADMIN_ROLE: &str = "cluster-admin";
const ADMIN_ROLE: &str = "admin";
const EDIT_ROLE: &str = "edit";
const VIEW_ROLE: &str = "view";
const CLUSTER_ADMIN_BINDING: &str = "cluster-admin";

const ADMIN_AGGREGATE_CHILD: &str = "system:aggregate-to-admin";
const EDIT_AGGREGATE_CHILD: &str = "system:aggregate-to-edit";
const VIEW_AGGREGATE_CHILD: &str = "system:aggregate-to-view";

const AGGREGATE_TO_ADMIN_LABEL: &str = "rbac.tugboat.cloud/aggregate-to-admin";
const AGGREGATE_TO_EDIT_LABEL: &str = "rbac.tugboat.cloud/aggregate-to-edit";
const AGGREGATE_TO_VIEW_LABEL: &str = "rbac.tugboat.cloud/aggregate-to-view";
const LABEL_TRUE: &str = "true";

pub(crate) async fn bootstrap_default_rbac(store: &ResourceStore) -> Result<(), Error> {
    store
        .put_if_not_exists(cluster_role(
            CLUSTER_ADMIN_ROLE,
            vec![policy_rule(&["*"], &["*"], &["*"])],
        ))
        .await?;

    for child in builtin_aggregate_children() {
        store.put_if_not_exists(child).await?;
    }

    for role in builtin_aggregated_cluster_roles() {
        ensure_aggregated_cluster_role(store, role).await?;
    }

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

async fn ensure_aggregated_cluster_role(
    store: &ResourceStore,
    desired: ClusterRole,
) -> Result<(), Error> {
    let name = desired
        .object_meta
        .as_ref()
        .and_then(|meta| meta.name.as_deref())
        .ok_or_else(|| Error::FieldMissing("metadata.name".to_string()))?
        .to_string();
    if store.put_if_not_exists(desired.clone()).await?.is_some() {
        return Ok(());
    }

    let Some(existing) = store.get::<ClusterRole>(None, &name).await? else {
        return Ok(());
    };
    let mut existing = existing.apply_revision();
    if apply_missing_aggregation_rule(&mut existing, desired.aggregation_rule) {
        store.put(existing).await?;
    }
    Ok(())
}

fn apply_missing_aggregation_rule(
    role: &mut ClusterRole,
    aggregation_rule: Option<AggregationRule>,
) -> bool {
    if role.aggregation_rule.is_some() {
        return false;
    }
    role.aggregation_rule = aggregation_rule;
    role.aggregation_rule.is_some()
}

fn builtin_aggregated_cluster_roles() -> Vec<ClusterRole> {
    vec![
        aggregated_cluster_role(ADMIN_ROLE, AGGREGATE_TO_ADMIN_LABEL, admin_rules()),
        aggregated_cluster_role(EDIT_ROLE, AGGREGATE_TO_EDIT_LABEL, edit_rules()),
        aggregated_cluster_role(VIEW_ROLE, AGGREGATE_TO_VIEW_LABEL, view_rules()),
    ]
}

fn builtin_aggregate_children() -> Vec<ClusterRole> {
    vec![
        aggregate_child(
            ADMIN_AGGREGATE_CHILD,
            AGGREGATE_TO_ADMIN_LABEL,
            admin_rules(),
        ),
        aggregate_child(EDIT_AGGREGATE_CHILD, AGGREGATE_TO_EDIT_LABEL, edit_rules()),
        aggregate_child(VIEW_AGGREGATE_CHILD, AGGREGATE_TO_VIEW_LABEL, view_rules()),
    ]
}

fn admin_rules() -> Vec<PolicyRule> {
    vec![
        policy_rule(
            &["core", "apps", "coordination", "snapshot"],
            &["*"],
            &["*"],
        ),
        policy_rule(&[RBAC_API_GROUP], &["roles", "rolebindings"], &["*"]),
    ]
}

fn edit_rules() -> Vec<PolicyRule> {
    vec![
        policy_rule(
            &["core", "apps", "coordination", "snapshot"],
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
    ]
}

fn view_rules() -> Vec<PolicyRule> {
    vec![
        policy_rule(
            &["core", "apps", "coordination", "snapshot"],
            &["*"],
            &["get", "list", "watch"],
        ),
        policy_rule(
            &[RBAC_API_GROUP],
            &["roles", "rolebindings"],
            &["get", "list", "watch"],
        ),
    ]
}

fn cluster_role(name: &str, rules: Vec<PolicyRule>) -> ClusterRole {
    ClusterRole {
        type_meta: Some(ClusterRole::type_meta()),
        object_meta: Some(cluster_object_meta(name)),
        rules,
        aggregation_rule: None,
    }
}

fn aggregated_cluster_role(
    name: &str,
    aggregate_label: &str,
    initial_rules: Vec<PolicyRule>,
) -> ClusterRole {
    ClusterRole {
        type_meta: Some(ClusterRole::type_meta()),
        object_meta: Some(cluster_object_meta(name)),
        rules: initial_rules,
        aggregation_rule: Some(AggregationRule {
            cluster_role_selectors: vec![LabelSelector {
                match_labels: HashMap::from([(
                    aggregate_label.to_string(),
                    LABEL_TRUE.to_string(),
                )]),
                match_expressions: Vec::new(),
            }],
        }),
    }
}

fn aggregate_child(name: &str, aggregate_label: &str, rules: Vec<PolicyRule>) -> ClusterRole {
    let mut meta = cluster_object_meta(name);
    meta.labels = HashMap::from([(aggregate_label.to_string(), LABEL_TRUE.to_string())]);
    ClusterRole {
        type_meta: Some(ClusterRole::type_meta()),
        object_meta: Some(meta),
        rules,
        aggregation_rule: None,
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
    use super::{
        ADMIN_AGGREGATE_CHILD, AGGREGATE_TO_ADMIN_LABEL, AGGREGATE_TO_EDIT_LABEL,
        AGGREGATE_TO_VIEW_LABEL, EDIT_AGGREGATE_CHILD, LABEL_TRUE, VIEW_AGGREGATE_CHILD,
        admin_rules, aggregate_child, aggregated_cluster_role, apply_missing_aggregation_rule,
        builtin_aggregate_children, cluster_role, cluster_role_binding, edit_rules, policy_rule,
        view_rules,
    };
    use tugboat_resources::ObjectMetaResource;

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
        assert!(role.aggregation_rule.is_none());
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

    #[test]
    fn aggregated_cluster_role_has_aggregation_rule_with_label_selector() {
        let role = aggregated_cluster_role("view", AGGREGATE_TO_VIEW_LABEL, view_rules());
        let aggregation_rule = role
            .aggregation_rule
            .expect("aggregation_rule should be set");
        assert_eq!(aggregation_rule.cluster_role_selectors.len(), 1);
        let selector = &aggregation_rule.cluster_role_selectors[0];
        assert_eq!(
            selector.match_labels.get(AGGREGATE_TO_VIEW_LABEL),
            Some(&LABEL_TRUE.to_string())
        );
        assert!(!role.rules.is_empty(), "initial rules should be present");
    }

    #[test]
    fn aggregate_child_carries_label_for_parent_selector() {
        let child = aggregate_child(VIEW_AGGREGATE_CHILD, AGGREGATE_TO_VIEW_LABEL, view_rules());
        assert_eq!(child.name(), Some(VIEW_AGGREGATE_CHILD));
        assert!(child.aggregation_rule.is_none());
        let labels = child
            .object_meta
            .as_ref()
            .map(|meta| &meta.labels)
            .expect("labels");
        assert_eq!(
            labels.get(AGGREGATE_TO_VIEW_LABEL),
            Some(&LABEL_TRUE.to_string())
        );
    }

    #[test]
    fn missing_aggregation_rule_is_added_without_replacing_rules() {
        let mut role = cluster_role("view", vec![policy_rule(&["core"], &["ships"], &["get"])]);
        let desired = aggregated_cluster_role("view", AGGREGATE_TO_VIEW_LABEL, view_rules());

        assert!(apply_missing_aggregation_rule(
            &mut role,
            desired.aggregation_rule
        ));
        assert!(role.aggregation_rule.is_some());
        assert_eq!(role.rules.len(), 1);
        assert_eq!(role.rules[0].resources, vec!["ships".to_string()]);
    }

    #[test]
    fn existing_aggregation_rule_is_preserved() {
        let mut role = aggregated_cluster_role("view", AGGREGATE_TO_VIEW_LABEL, view_rules());
        let original = role.aggregation_rule.clone();
        let replacement = aggregated_cluster_role("view", AGGREGATE_TO_ADMIN_LABEL, admin_rules());

        assert!(!apply_missing_aggregation_rule(
            &mut role,
            replacement.aggregation_rule
        ));
        assert_eq!(role.aggregation_rule, original);
    }

    #[test]
    fn builtin_children_cover_admin_edit_view_with_matching_labels() {
        let children = builtin_aggregate_children();
        assert_eq!(children.len(), 3);

        let by_name: std::collections::HashMap<String, &_> = children
            .iter()
            .filter_map(|child| child.name().map(|name| (name.to_string(), child)))
            .collect();

        let admin = by_name.get(ADMIN_AGGREGATE_CHILD).expect("admin child");
        let edit = by_name.get(EDIT_AGGREGATE_CHILD).expect("edit child");
        let view = by_name.get(VIEW_AGGREGATE_CHILD).expect("view child");

        let admin_labels = admin
            .object_meta
            .as_ref()
            .map(|m| &m.labels)
            .expect("labels");
        let edit_labels = edit
            .object_meta
            .as_ref()
            .map(|m| &m.labels)
            .expect("labels");
        let view_labels = view
            .object_meta
            .as_ref()
            .map(|m| &m.labels)
            .expect("labels");

        assert_eq!(
            admin_labels.get(AGGREGATE_TO_ADMIN_LABEL),
            Some(&LABEL_TRUE.to_string())
        );
        assert_eq!(
            edit_labels.get(AGGREGATE_TO_EDIT_LABEL),
            Some(&LABEL_TRUE.to_string())
        );
        assert_eq!(
            view_labels.get(AGGREGATE_TO_VIEW_LABEL),
            Some(&LABEL_TRUE.to_string())
        );

        assert_eq!(admin.rules, admin_rules());
        assert_eq!(edit.rules, edit_rules());
        assert_eq!(view.rules, view_rules());
    }
}
