use crate::base::TugboatController;
use crate::error::ControllerError;
use std::collections::HashMap;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::authorization::v1::{
    AggregationRule, ClusterRole, LabelSelector, LabelSelectorRequirement, PolicyRule,
};

#[derive(Clone)]
struct AggregatedClusterRoleReconciler {
    client: TugboatClient,
}

pub(crate) struct AggregatedClusterRoleController {
    controller: Controller<ClusterRole>,
    reconciler: AggregatedClusterRoleReconciler,
}

impl AggregatedClusterRoleController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: AggregatedClusterRoleReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for AggregatedClusterRoleController {
    fn name(&self) -> &str {
        "aggregated-clusterrole"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<ClusterRole> for AggregatedClusterRoleReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<ClusterRole>) -> Result<Action, Self::Error> {
        let _ = match event {
            ReconcileEvent::Applied(role) | ReconcileEvent::Deleted(role) => role,
        };
        self.reconcile_all().await
    }
}

impl AggregatedClusterRoleReconciler {
    async fn reconcile_all(&self) -> Result<Action, ControllerError> {
        let api: Api<ClusterRole> = Api::all(self.client.clone());
        // TODO: this lists every ClusterRole on every event. With many roles
        // and a busy watch stream this is O(n) per event. Once label selectors
        // land on list, scope this to parents (aggregation_rule set) and
        // children matching any configured selector.
        let cluster_roles = api.list().await?;

        let parents: Vec<&ClusterRole> = cluster_roles
            .iter()
            .filter(|role| role.aggregation_rule.is_some())
            .collect();
        if parents.is_empty() {
            return Ok(Action::await_change());
        }

        let children: Vec<&ClusterRole> = cluster_roles
            .iter()
            .filter(|role| role.aggregation_rule.is_none())
            .collect();

        for parent in parents {
            let Some(name) = parent.name().map(str::to_string) else {
                continue;
            };
            let aggregation_rule = parent
                .aggregation_rule
                .as_ref()
                .expect("filtered to aggregation_rule parents above");
            let new_rules = aggregate_rules(aggregation_rule, &children);
            if rules_equivalent(&parent.rules, &new_rules) {
                continue;
            }
            let mut updated = parent.clone();
            updated.rules = new_rules;
            match api.replace(&name, updated).await {
                Ok(_) => {}
                Err(tugboat_client::Error::Api(status))
                    if status.code == 404 || status.code == 409 =>
                {
                    tracing::debug!(
                        "Skipping aggregated ClusterRole update for '{name}': {}",
                        status.message
                    );
                }
                Err(err) => return Err(err.into()),
            }
        }

        Ok(Action::await_change())
    }
}

/// Compute the union of rules from children whose labels match any selector in
/// `aggregation_rule`. The result is deterministic (children are visited in the
/// order returned by `list`) and deduplicated on the canonical `(api_groups,
/// resources, resource_names, verbs)` tuple.
fn aggregate_rules(
    aggregation_rule: &AggregationRule,
    children: &[&ClusterRole],
) -> Vec<PolicyRule> {
    let mut aggregated: Vec<PolicyRule> = Vec::new();
    let mut seen: std::collections::HashSet<RuleKey> = std::collections::HashSet::new();

    for child in children {
        let labels = child
            .object_meta()
            .as_ref()
            .map(|meta| &meta.labels)
            .cloned()
            .unwrap_or_default();
        if !selectors_match(&aggregation_rule.cluster_role_selectors, &labels) {
            continue;
        }
        for rule in &child.rules {
            let normalized = normalize_rule(rule);
            let key = RuleKey::from(&normalized);
            if seen.insert(key) {
                aggregated.push(normalized);
            }
        }
    }

    aggregated
}

fn selectors_match(selectors: &[LabelSelector], labels: &HashMap<String, String>) -> bool {
    if selectors.is_empty() {
        return false;
    }
    selectors
        .iter()
        .any(|selector| selector_matches(selector, labels))
}

/// A selector with no `match_labels` *and* no `match_expressions` matches
/// nothing — see proto comment on [`LabelSelector`]. Otherwise all entries are
/// ANDed: every match_label and every match_expression must be satisfied.
fn selector_matches(selector: &LabelSelector, labels: &HashMap<String, String>) -> bool {
    if selector.match_labels.is_empty() && selector.match_expressions.is_empty() {
        return false;
    }
    let labels_ok = selector
        .match_labels
        .iter()
        .all(|(key, value)| labels.get(key).is_some_and(|label| label == value));
    if !labels_ok {
        return false;
    }
    selector
        .match_expressions
        .iter()
        .all(|requirement| requirement_matches(requirement, labels))
}

fn requirement_matches(
    requirement: &LabelSelectorRequirement,
    labels: &HashMap<String, String>,
) -> bool {
    match requirement.operator.as_str() {
        "In" => labels
            .get(&requirement.key)
            .is_some_and(|value| requirement.values.iter().any(|expected| expected == value)),
        "NotIn" => match labels.get(&requirement.key) {
            None => true,
            Some(value) => !requirement.values.iter().any(|expected| expected == value),
        },
        "Exists" => labels.contains_key(&requirement.key),
        "DoesNotExist" => !labels.contains_key(&requirement.key),
        // Reject malformed requirements (unknown operator, In/NotIn with empty
        // values list). A misconfigured selector must not silently match every
        // ClusterRole.
        _ => false,
    }
}

fn normalize_rule(rule: &PolicyRule) -> PolicyRule {
    PolicyRule {
        api_groups: sorted_unique(&rule.api_groups),
        resources: sorted_unique(&rule.resources),
        resource_names: sorted_unique(&rule.resource_names),
        verbs: sorted_unique(&rule.verbs),
    }
}

fn sorted_unique(values: &[String]) -> Vec<String> {
    let mut deduped: Vec<String> = values.to_vec();
    deduped.sort();
    deduped.dedup();
    deduped
}

#[derive(Eq, Hash, PartialEq)]
struct RuleKey {
    api_groups: Vec<String>,
    resources: Vec<String>,
    resource_names: Vec<String>,
    verbs: Vec<String>,
}

impl From<&PolicyRule> for RuleKey {
    fn from(rule: &PolicyRule) -> Self {
        Self {
            api_groups: rule.api_groups.clone(),
            resources: rule.resources.clone(),
            resource_names: rule.resource_names.clone(),
            verbs: rule.verbs.clone(),
        }
    }
}

fn rules_equivalent(current: &[PolicyRule], desired: &[PolicyRule]) -> bool {
    if current.len() != desired.len() {
        return false;
    }
    let current_normalized: Vec<PolicyRule> = current.iter().map(normalize_rule).collect();
    let desired_normalized: Vec<PolicyRule> = desired.iter().map(normalize_rule).collect();
    let current_keys: std::collections::HashSet<RuleKey> =
        current_normalized.iter().map(RuleKey::from).collect();
    let desired_keys: std::collections::HashSet<RuleKey> =
        desired_normalized.iter().map(RuleKey::from).collect();
    current_keys == desired_keys
}

#[cfg(test)]
mod tests {
    use super::{
        aggregate_rules, normalize_rule, requirement_matches, rules_equivalent, selector_matches,
        selectors_match, sorted_unique,
    };
    use std::collections::HashMap;
    use tugboat_resources::manifests::authorization::v1::{
        AggregationRule, ClusterRole, LabelSelector, LabelSelectorRequirement, PolicyRule,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    fn child(name: &str, labels: &[(&str, &str)], rules: Vec<PolicyRule>) -> ClusterRole {
        ClusterRole {
            type_meta: None,
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                labels: labels
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                    .collect(),
                ..Default::default()
            }),
            rules,
            aggregation_rule: None,
        }
    }

    fn rule(api_groups: &[&str], resources: &[&str], verbs: &[&str]) -> PolicyRule {
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

    fn selector(entries: &[(&str, &str)]) -> LabelSelector {
        LabelSelector {
            match_labels: entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
            match_expressions: Vec::new(),
        }
    }

    fn requirement(key: &str, op: &str, values: &[&str]) -> LabelSelectorRequirement {
        LabelSelectorRequirement {
            key: key.to_string(),
            operator: op.to_string(),
            values: values.iter().map(|v| (*v).to_string()).collect(),
        }
    }

    #[test]
    fn empty_match_labels_does_not_match_anything() {
        let labels = HashMap::from([(
            "rbac.tugboat.cloud/aggregate-to-view".to_string(),
            "true".to_string(),
        )]);
        let empty_selector = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: Vec::new(),
        };
        assert!(!selector_matches(&empty_selector, &labels));
    }

    #[test]
    fn match_expressions_in_operator_requires_value_in_list() {
        let labels = HashMap::from([("tier".to_string(), "view".to_string())]);
        assert!(requirement_matches(
            &requirement("tier", "In", &["view", "edit"]),
            &labels
        ));
        assert!(!requirement_matches(
            &requirement("tier", "In", &["admin"]),
            &labels
        ));
    }

    #[test]
    fn match_expressions_notin_operator_excludes_value() {
        let labels = HashMap::from([("tier".to_string(), "view".to_string())]);
        assert!(!requirement_matches(
            &requirement("tier", "NotIn", &["view"]),
            &labels
        ));
        assert!(requirement_matches(
            &requirement("tier", "NotIn", &["edit"]),
            &labels
        ));
        // Key missing → NotIn passes (matches Kubernetes semantics).
        assert!(requirement_matches(
            &requirement("absent", "NotIn", &["x"]),
            &labels
        ));
    }

    #[test]
    fn match_expressions_exists_and_does_not_exist() {
        let labels = HashMap::from([("tier".to_string(), "view".to_string())]);
        assert!(requirement_matches(
            &requirement("tier", "Exists", &[]),
            &labels
        ));
        assert!(!requirement_matches(
            &requirement("absent", "Exists", &[]),
            &labels
        ));
        assert!(requirement_matches(
            &requirement("absent", "DoesNotExist", &[]),
            &labels
        ));
        assert!(!requirement_matches(
            &requirement("tier", "DoesNotExist", &[]),
            &labels
        ));
    }

    #[test]
    fn unknown_operator_does_not_silently_match() {
        let labels = HashMap::from([("tier".to_string(), "view".to_string())]);
        assert!(!requirement_matches(
            &requirement("tier", "GreaterThan", &["a"]),
            &labels
        ));
    }

    #[test]
    fn selector_combines_match_labels_and_match_expressions() {
        let labels = HashMap::from([
            ("tier".to_string(), "view".to_string()),
            ("team".to_string(), "platform".to_string()),
        ]);
        let selector = LabelSelector {
            match_labels: HashMap::from([("tier".to_string(), "view".to_string())]),
            match_expressions: vec![requirement("team", "In", &["platform", "infra"])],
        };
        assert!(selector_matches(&selector, &labels));

        let selector_team_mismatch = LabelSelector {
            match_labels: HashMap::from([("tier".to_string(), "view".to_string())]),
            match_expressions: vec![requirement("team", "In", &["infra"])],
        };
        assert!(!selector_matches(&selector_team_mismatch, &labels));
    }

    #[test]
    fn selector_with_only_match_expressions_works() {
        let labels = HashMap::from([("tier".to_string(), "view".to_string())]);
        let selector = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![requirement("tier", "Exists", &[])],
        };
        assert!(selector_matches(&selector, &labels));
    }

    #[test]
    fn match_labels_requires_all_entries_to_match() {
        let labels = HashMap::from([
            ("tier".to_string(), "view".to_string()),
            ("team".to_string(), "platform".to_string()),
        ]);
        let single = selector(&[("tier", "view")]);
        let both = selector(&[("tier", "view"), ("team", "platform")]);
        let mismatched = selector(&[("tier", "view"), ("team", "infra")]);

        assert!(selector_matches(&single, &labels));
        assert!(selector_matches(&both, &labels));
        assert!(!selector_matches(&mismatched, &labels));
    }

    #[test]
    fn selectors_are_union_across_entries() {
        let labels = HashMap::from([("team".to_string(), "infra".to_string())]);
        let selectors = vec![
            selector(&[("tier", "view")]),
            selector(&[("team", "infra")]),
        ];
        assert!(selectors_match(&selectors, &labels));
    }

    #[test]
    fn empty_selector_list_matches_nothing() {
        let labels = HashMap::from([("team".to_string(), "infra".to_string())]);
        assert!(!selectors_match(&[], &labels));
    }

    #[test]
    fn aggregates_matching_children_and_deduplicates_rules() {
        let aggregation_rule = AggregationRule {
            cluster_role_selectors: vec![selector(&[(
                "rbac.tugboat.cloud/aggregate-to-view",
                "true",
            )])],
        };
        let matching = child(
            "system:aggregate-to-view",
            &[("rbac.tugboat.cloud/aggregate-to-view", "true")],
            vec![rule(&["core"], &["ships"], &["get", "list", "watch"])],
        );
        let also_matching = child(
            "extension:aggregate-to-view",
            &[("rbac.tugboat.cloud/aggregate-to-view", "true")],
            vec![
                rule(&["core"], &["ships"], &["get", "list", "watch"]),
                rule(&["core"], &["secrets"], &["get"]),
            ],
        );
        let unrelated = child(
            "unrelated",
            &[("rbac.tugboat.cloud/aggregate-to-edit", "true")],
            vec![rule(&["core"], &["ships"], &["delete"])],
        );

        let children = vec![&matching, &also_matching, &unrelated];
        let aggregated = aggregate_rules(&aggregation_rule, &children);

        assert_eq!(aggregated.len(), 2);
        assert!(
            aggregated
                .iter()
                .any(|r| r.resources == vec!["ships".to_string()])
        );
        assert!(
            aggregated
                .iter()
                .any(|r| r.resources == vec!["secrets".to_string()])
        );
    }

    #[test]
    fn equivalent_rule_sets_are_reported_equal_regardless_of_order() {
        let a = vec![
            rule(&["core"], &["ships"], &["get", "list"]),
            rule(&["core"], &["secrets"], &["get"]),
        ];
        let b = vec![
            rule(&["core"], &["secrets"], &["get"]),
            rule(&["core"], &["ships"], &["list", "get"]),
        ];

        assert!(rules_equivalent(&a, &b));
    }

    #[test]
    fn differing_rule_sets_are_not_equivalent() {
        let a = vec![rule(&["core"], &["ships"], &["get"])];
        let b = vec![rule(&["core"], &["ships"], &["get", "list"])];
        assert!(!rules_equivalent(&a, &b));
    }

    #[test]
    fn normalize_rule_sorts_and_dedupes_each_field() {
        let normalized = normalize_rule(&PolicyRule {
            api_groups: vec!["core".to_string(), "apps".to_string(), "core".to_string()],
            resources: vec!["ships".to_string()],
            resource_names: Vec::new(),
            verbs: vec!["watch".to_string(), "get".to_string(), "watch".to_string()],
        });
        assert_eq!(
            normalized.api_groups,
            vec!["apps".to_string(), "core".to_string()]
        );
        assert_eq!(
            normalized.verbs,
            vec!["get".to_string(), "watch".to_string()]
        );
    }

    #[test]
    fn sorted_unique_sorts_and_dedupes_values() {
        let result = sorted_unique(&[
            "b".to_string(),
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ]);
        assert_eq!(
            result,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }
}
