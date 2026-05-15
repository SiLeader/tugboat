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
use tugboat_resources::manifests::core::v1::Node;

pub struct UnschedulableFilter;

impl FilterPlugin for UnschedulableFilter {
    fn name(&self) -> &str {
        "Unschedulable"
    }

    fn filter(&self, _ctx: &SchedulingContext, node: &Node) -> FilterResult {
        if node
            .spec
            .as_ref()
            .and_then(|spec| spec.unschedulable)
            .unwrap_or(false)
        {
            FilterResult::Reject("node is marked unschedulable".to_string())
        } else {
            FilterResult::Accept
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use tugboat_resources::manifests::core::v1::{NodeSpec, Ship, ShipClass};

    fn context() -> SchedulingContext {
        SchedulingContext {
            ship: Ship::default(),
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
        }
    }

    #[test]
    fn accepts_schedulable_node() {
        let filter = UnschedulableFilter;
        let node = Node {
            spec: Some(NodeSpec {
                unschedulable: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(matches!(
            filter.filter(&context(), &node),
            FilterResult::Accept
        ));
    }

    #[test]
    fn rejects_unschedulable_node() {
        let filter = UnschedulableFilter;
        let node = Node {
            spec: Some(NodeSpec {
                unschedulable: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        match filter.filter(&context(), &node) {
            FilterResult::Reject(reason) => assert!(reason.contains("unschedulable")),
            FilterResult::Accept => panic!("expected node to be rejected"),
        }
    }
}
