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

use crate::framework::{FilterPlugin, FilterResult, SchedulingContext, ScorePlugin, ScoreResult};
use crate::plugins::selectors::label_selector_matches;
use std::collections::HashMap;
use tugboat_resources::manifests::core::v1::{Node, TopologySpreadConstraint};

const DO_NOT_SCHEDULE: &str = "DoNotSchedule";

pub struct TopologySpreadFilter;

impl FilterPlugin for TopologySpreadFilter {
    fn name(&self) -> &str {
        "TopologySpread"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let Some(spec) = ctx.ship.spec.as_ref() else {
            return FilterResult::Accept;
        };

        for constraint in &spec.topology_spread_constraints {
            if constraint.when_unsatisfiable != DO_NOT_SCHEDULE {
                continue;
            }
            let Some(skew) = skew_if_scheduled(ctx, node, constraint) else {
                return FilterResult::Reject(format!(
                    "topology spread key '{}' is missing on the candidate node",
                    constraint.topology_key
                ));
            };
            if skew > i64::from(constraint.max_skew) {
                return FilterResult::Reject(format!(
                    "topology spread constraint '{}' would exceed maxSkew {}",
                    constraint.topology_key, constraint.max_skew
                ));
            }
        }

        FilterResult::Accept
    }
}

pub struct TopologySpreadScorer;

impl ScorePlugin for TopologySpreadScorer {
    fn name(&self) -> &str {
        "TopologySpread"
    }

    fn score(&self, ctx: &SchedulingContext, node: &Node) -> ScoreResult {
        let Some(spec) = ctx.ship.spec.as_ref() else {
            return ScoreResult::Skip;
        };
        if spec.topology_spread_constraints.is_empty() {
            return ScoreResult::Skip;
        }

        let mut total = 0i64;
        let mut count = 0i64;
        for constraint in &spec.topology_spread_constraints {
            count += 1;
            let Some(skew) = skew_if_scheduled(ctx, node, constraint) else {
                continue;
            };
            let over = skew.saturating_sub(i64::from(constraint.max_skew));
            total += (100 - over * 50).clamp(0, 100);
        }

        if count == 0 {
            ScoreResult::Skip
        } else {
            ScoreResult::Score(total / count)
        }
    }
}

fn skew_if_scheduled(
    ctx: &SchedulingContext,
    node: &Node,
    constraint: &TopologySpreadConstraint,
) -> Option<i64> {
    let candidate_value = node
        .object_meta
        .as_ref()
        .and_then(|meta| meta.labels.get(&constraint.topology_key))?;
    let mut buckets = topology_buckets(ctx, constraint);
    if buckets.is_empty() {
        return None;
    }
    *buckets.entry(candidate_value.clone()).or_insert(0) += 1;
    let min_count = buckets.values().min().copied().unwrap_or(0);
    let candidate_count = buckets.get(candidate_value).copied().unwrap_or(0);
    Some(candidate_count - min_count)
}

fn topology_buckets(
    ctx: &SchedulingContext,
    constraint: &TopologySpreadConstraint,
) -> HashMap<String, i64> {
    let mut buckets = HashMap::new();
    let node_map: HashMap<&str, &Node> = ctx
        .all_nodes
        .iter()
        .filter_map(|node| {
            node.object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref())
                .map(|name| (name, node))
        })
        .collect();

    for node in &ctx.all_nodes {
        if let Some(value) = node
            .object_meta
            .as_ref()
            .and_then(|meta| meta.labels.get(&constraint.topology_key))
        {
            buckets.entry(value.clone()).or_insert(0);
        }
    }

    for ship in &ctx.all_ships {
        let Some(node_name) = ship
            .spec
            .as_ref()
            .and_then(|ship_spec| ship_spec.node_name.as_deref())
            .filter(|node_name| !node_name.is_empty())
        else {
            continue;
        };
        let labels_match = ship.object_meta.as_ref().is_some_and(|meta| {
            label_selector_matches(constraint.label_selector.as_ref(), &meta.labels)
        });
        if !labels_match {
            continue;
        }

        let Some(value) = node_map
            .get(node_name)
            .and_then(|node| node.object_meta.as_ref())
            .and_then(|meta| meta.labels.get(&constraint.topology_key))
        else {
            continue;
        };
        *buckets.entry(value.clone()).or_insert(0) += 1;
    }
    buckets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::{Ship, ShipClass, ShipSpec};
    use tugboat_resources::manifests::meta::v1::{LabelSelector, ObjectMeta};

    #[test]
    fn do_not_schedule_rejects_excessive_skew() {
        let zone_a = Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-a".to_string()),
                labels: HashMap::from([("zone".to_string(), "a".to_string())]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let zone_b = Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-b".to_string()),
                labels: HashMap::from([("zone".to_string(), "b".to_string())]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let peer = Ship {
            object_meta: Some(ObjectMeta {
                namespace: Some("default".to_string()),
                labels: HashMap::from([("app".to_string(), "api".to_string())]),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ctx = SchedulingContext {
            ship: Ship {
                spec: Some(ShipSpec {
                    topology_spread_constraints: vec![TopologySpreadConstraint {
                        max_skew: 1,
                        topology_key: "zone".to_string(),
                        when_unsatisfiable: "DoNotSchedule".to_string(),
                        label_selector: Some(LabelSelector {
                            match_labels: HashMap::from([("app".to_string(), "api".to_string())]),
                            ..Default::default()
                        }),
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
            ship_class: ShipClass::default(),
            all_cluster_network_classes: Vec::new(),
            all_network_classes: Vec::new(),
            all_runtime_classes: Vec::new(),
            all_ships: vec![peer],
            all_nodes: vec![zone_a.clone(), zone_b],
            all_ship_classes: Vec::new(),
            all_persistent_volume_claims: Vec::new(),
            all_persistent_volumes: Vec::new(),
            all_storage_classes: Vec::new(),
        };

        assert!(matches!(
            TopologySpreadFilter.filter(&ctx, &zone_a),
            FilterResult::Reject(_)
        ));
    }
}
