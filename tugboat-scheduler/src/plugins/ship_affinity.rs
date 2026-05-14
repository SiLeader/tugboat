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

use crate::framework::{FilterPlugin, FilterResult, SchedulingContext};
use crate::plugins::selectors::label_selector_matches;
use std::collections::HashMap;
use tugboat_resources::manifests::core::v1::{Node, Ship, ShipAffinityTerm};

pub struct ShipAffinityFilter;

impl FilterPlugin for ShipAffinityFilter {
    fn name(&self) -> &str {
        "ShipAffinity"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let Some(affinity) = ctx
            .ship
            .spec
            .as_ref()
            .and_then(|spec| spec.affinity.as_ref())
        else {
            return FilterResult::Accept;
        };

        let node_labels = node
            .object_meta
            .as_ref()
            .map(|meta| &meta.labels)
            .cloned()
            .unwrap_or_default();

        if let Some(ship_affinity) = affinity.ship_affinity.as_ref() {
            for term in &ship_affinity.required_during_scheduling {
                if !matches_required_affinity_term(ctx, term, &node_labels) {
                    return FilterResult::Reject(format!(
                        "required Ship affinity is not satisfied for topology key '{}'",
                        term.topology_key
                    ));
                }
            }
        }

        if let Some(ship_anti_affinity) = affinity.ship_anti_affinity.as_ref() {
            for term in &ship_anti_affinity.required_during_scheduling {
                if matches_required_affinity_term(ctx, term, &node_labels) {
                    return FilterResult::Reject(format!(
                        "required Ship anti-affinity is not satisfied for topology key '{}'",
                        term.topology_key
                    ));
                }
            }
        }

        FilterResult::Accept
    }
}

fn matches_required_affinity_term(
    ctx: &SchedulingContext,
    term: &ShipAffinityTerm,
    candidate_labels: &HashMap<String, String>,
) -> bool {
    let Some(candidate_value) = candidate_labels.get(&term.topology_key) else {
        return false;
    };

    matching_scheduled_ships(ctx, term).any(|peer| {
        let Some(peer_node_name) = peer
            .spec
            .as_ref()
            .and_then(|spec| spec.node_name.as_deref())
        else {
            return false;
        };
        ctx.all_nodes
            .iter()
            .find(|node| {
                node.object_meta
                    .as_ref()
                    .and_then(|meta| meta.name.as_deref())
                    == Some(peer_node_name)
            })
            .and_then(|node| node.object_meta.as_ref())
            .and_then(|meta| meta.labels.get(&term.topology_key))
            == Some(candidate_value)
    })
}

fn matching_scheduled_ships<'a>(
    ctx: &'a SchedulingContext,
    term: &'a ShipAffinityTerm,
) -> impl Iterator<Item = &'a Ship> {
    let current_namespace = ctx.ship_namespace().to_string();
    ctx.all_ships.iter().filter(move |ship| {
        ship.spec
            .as_ref()
            .and_then(|spec| spec.node_name.as_deref())
            .is_some_and(|node_name| !node_name.is_empty())
            && namespace_matches(ship, &current_namespace, &term.namespaces)
            && ship.object_meta.as_ref().is_some_and(|meta| {
                label_selector_matches(term.label_selector.as_ref(), &meta.labels)
            })
    })
}

fn namespace_matches(ship: &Ship, current_namespace: &str, namespaces: &[String]) -> bool {
    let namespace = ship
        .object_meta
        .as_ref()
        .and_then(|meta| meta.namespace.as_deref())
        .unwrap_or("default");
    if namespaces.is_empty() {
        namespace == current_namespace
    } else {
        namespaces.iter().any(|item| item == namespace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::{Affinity, ShipAntiAffinity, ShipClass, ShipSpec};
    use tugboat_resources::manifests::meta::v1::{LabelSelector, ObjectMeta};

    #[test]
    fn anti_affinity_rejects_node_in_matching_peer_topology() {
        let peer = Ship {
            object_meta: Some(ObjectMeta {
                namespace: Some("default".to_string()),
                labels: HashMap::from([("app".to_string(), "db".to_string())]),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let node_a = Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-a".to_string()),
                labels: HashMap::from([(
                    "topology.tugboat.cloud/zone".to_string(),
                    "zone-a".to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let candidate = Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-b".to_string()),
                labels: HashMap::from([(
                    "topology.tugboat.cloud/zone".to_string(),
                    "zone-a".to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ctx = SchedulingContext {
            ship: Ship {
                object_meta: Some(ObjectMeta {
                    namespace: Some("default".to_string()),
                    ..Default::default()
                }),
                spec: Some(ShipSpec {
                    affinity: Some(Affinity {
                        ship_anti_affinity: Some(ShipAntiAffinity {
                            required_during_scheduling: vec![ShipAffinityTerm {
                                label_selector: Some(LabelSelector {
                                    match_labels: HashMap::from([(
                                        "app".to_string(),
                                        "db".to_string(),
                                    )]),
                                    ..Default::default()
                                }),
                                topology_key: "topology.tugboat.cloud/zone".to_string(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ship_class: ShipClass::default(),
            all_cluster_network_classes: Vec::new(),
            all_network_classes: Vec::new(),
            all_runtime_classes: Vec::new(),
            all_ships: vec![peer],
            all_nodes: vec![node_a],
            all_ship_classes: Vec::new(),
            all_persistent_volume_claims: Vec::new(),
            all_persistent_volumes: Vec::new(),
            all_storage_classes: Vec::new(),
        };

        assert!(matches!(
            ShipAffinityFilter.filter(&ctx, &candidate),
            FilterResult::Reject(_)
        ));
    }
}
