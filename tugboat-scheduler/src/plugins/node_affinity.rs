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
use crate::plugins::selectors::{first_unsatisfied_node_selector_key, node_selector_matches};
use tugboat_resources::manifests::core::v1::Node;

pub struct NodeAffinityFilter;

impl FilterPlugin for NodeAffinityFilter {
    fn name(&self) -> &str {
        "NodeAffinity"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let Some(selector) = ctx
            .ship
            .spec
            .as_ref()
            .and_then(|spec| spec.affinity.as_ref())
            .and_then(|affinity| affinity.node_affinity.as_ref())
            .and_then(|node_affinity| node_affinity.required_during_scheduling.as_ref())
        else {
            return FilterResult::Accept;
        };

        let labels = node
            .object_meta
            .as_ref()
            .map(|meta| &meta.labels)
            .cloned()
            .unwrap_or_default();
        if node_selector_matches(selector, &labels) {
            return FilterResult::Accept;
        }

        let key = first_unsatisfied_node_selector_key(selector, &labels)
            .unwrap_or_else(|| "<unknown>".to_string());
        FilterResult::Reject(format!(
            "Ship node affinity is not satisfied for key '{key}'"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::{
        Affinity, NodeAffinity, NodeSelector, NodeSelectorRequirement, NodeSelectorTerm, Ship,
        ShipClass, ShipSpec,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn rejects_node_that_does_not_match_required_affinity() {
        let ctx = SchedulingContext {
            ship: Ship {
                spec: Some(ShipSpec {
                    affinity: Some(Affinity {
                        node_affinity: Some(NodeAffinity {
                            required_during_scheduling: Some(NodeSelector {
                                node_selector_terms: vec![NodeSelectorTerm {
                                    match_expressions: vec![NodeSelectorRequirement {
                                        key: "topology.tugboat.cloud/zone".to_string(),
                                        operator: "In".to_string(),
                                        values: vec!["zone-a".to_string()],
                                    }],
                                    ..Default::default()
                                }],
                            }),
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
            all_ships: Vec::new(),
            all_nodes: Vec::new(),
            all_ship_classes: Vec::new(),
            all_persistent_volume_claims: Vec::new(),
            all_persistent_volumes: Vec::new(),
            all_storage_classes: Vec::new(),
        };
        let node = Node {
            object_meta: Some(ObjectMeta {
                labels: HashMap::from([(
                    "topology.tugboat.cloud/zone".to_string(),
                    "zone-b".to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(matches!(
            NodeAffinityFilter.filter(&ctx, &node),
            FilterResult::Reject(_)
        ));
    }
}
