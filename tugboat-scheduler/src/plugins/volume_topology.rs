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
use crate::plugins::selectors::{
    first_unsatisfied_node_selector_key, node_selector_matches, topology_selector_terms_match,
};
use tugboat_resources::manifests::core::v1::Node;

pub struct VolumeTopologyFilter;

impl FilterPlugin for VolumeTopologyFilter {
    fn name(&self) -> &str {
        "VolumeTopology"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let labels = node
            .object_meta
            .as_ref()
            .map(|meta| &meta.labels)
            .cloned()
            .unwrap_or_default();

        // Check node affinity of bound PVs
        for pv in ctx.ship_bound_persistent_volumes() {
            let Some(selector) = pv
                .spec
                .as_ref()
                .and_then(|spec| spec.node_affinity.as_ref())
                .and_then(|affinity| affinity.required.as_ref())
            else {
                continue;
            };

            if node_selector_matches(selector, &labels) {
                continue;
            }

            let pv_name = pv
                .object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref())
                .unwrap_or("<unknown>");
            let key = first_unsatisfied_node_selector_key(selector, &labels)
                .unwrap_or_else(|| "<unknown>".to_string());
            return FilterResult::Reject(format!(
                "PersistentVolume '{pv_name}' node affinity is not satisfied for key '{key}'"
            ));
        }

        // Check allowedTopologies of StorageClasses for unbound PVCs
        for pvc in ctx.ship_persistent_volume_claims() {
            if !ctx.is_unbound_wffc_claim(pvc) {
                continue;
            }
            let Some(storage_class_name) = pvc
                .spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref())
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let Some(sc) = ctx.find_storage_class(storage_class_name) else {
                continue;
            };
            let Some(sc_spec) = sc.spec.as_ref() else {
                continue;
            };

            if !topology_selector_terms_match(&sc_spec.allowed_topologies, &labels) {
                let pvc_name = pvc
                    .object_meta
                    .as_ref()
                    .and_then(|meta| meta.name.as_deref())
                    .unwrap_or("<unknown>");
                return FilterResult::Reject(format!(
                    "PersistentVolumeClaim '{pvc_name}' StorageClass '{storage_class_name}' allowed topologies are not satisfied"
                ));
            }
        }

        FilterResult::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::PersistentVolumeClaimVolumeSource;
    use tugboat_resources::manifests::core::v1::{
        NodeSelector, NodeSelectorRequirement, NodeSelectorTerm, PersistentVolume,
        PersistentVolumeClaim, PersistentVolumeClaimSpec, PersistentVolumeSpec, Ship, ShipClass,
        ShipSpec, ShipVolume, VolumeNodeAffinity,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn rejects_pv_node_affinity_mismatch() {
        let ctx = SchedulingContext {
            ship: Ship {
                object_meta: Some(ObjectMeta {
                    namespace: Some("default".to_string()),
                    ..Default::default()
                }),
                spec: Some(ShipSpec {
                    volumes: vec![ShipVolume {
                        name: "data".to_string(),
                        persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                            claim_name: "data".to_string(),
                        }),
                        ..Default::default()
                    }],
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
            all_persistent_volume_claims: vec![PersistentVolumeClaim {
                object_meta: Some(ObjectMeta {
                    name: Some("data".to_string()),
                    namespace: Some("default".to_string()),
                    ..Default::default()
                }),
                spec: Some(PersistentVolumeClaimSpec {
                    volume_name: Some("pv-data".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            all_persistent_volumes: vec![PersistentVolume {
                object_meta: Some(ObjectMeta {
                    name: Some("pv-data".to_string()),
                    ..Default::default()
                }),
                spec: Some(PersistentVolumeSpec {
                    node_affinity: Some(VolumeNodeAffinity {
                        required: Some(NodeSelector {
                            node_selector_terms: vec![NodeSelectorTerm {
                                match_expressions: vec![NodeSelectorRequirement {
                                    key: "topology.tugboat.cloud/zone".to_string(),
                                    operator: "In".to_string(),
                                    values: vec!["zone-a".to_string()],
                                }],
                                ..Default::default()
                            }],
                        }),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
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
            VolumeTopologyFilter.filter(&ctx, &node),
            FilterResult::Reject(reason) if reason.contains("pv-data")
        ));
    }

    #[test]
    fn rejects_unbound_pvc_storage_class_topology_mismatch() {
        use tugboat_resources::manifests::core::v1::{
            StorageClass, StorageClassSpec, TopologySelectorLabelRequirement, TopologySelectorTerm,
        };

        let ctx = SchedulingContext {
            ship: Ship {
                object_meta: Some(ObjectMeta {
                    namespace: Some("default".to_string()),
                    ..Default::default()
                }),
                spec: Some(ShipSpec {
                    volumes: vec![ShipVolume {
                        name: "data".to_string(),
                        persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                            claim_name: "data".to_string(),
                        }),
                        ..Default::default()
                    }],
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
            all_persistent_volume_claims: vec![PersistentVolumeClaim {
                object_meta: Some(ObjectMeta {
                    name: Some("data".to_string()),
                    namespace: Some("default".to_string()),
                    ..Default::default()
                }),
                spec: Some(PersistentVolumeClaimSpec {
                    storage_class_name: Some("fast".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            all_persistent_volumes: Vec::new(),
            all_storage_classes: vec![StorageClass {
                object_meta: Some(ObjectMeta {
                    name: Some("fast".to_string()),
                    ..Default::default()
                }),
                spec: Some(StorageClassSpec {
                    volume_binding_mode: Some("WaitForFirstConsumer".to_string()),
                    allowed_topologies: vec![TopologySelectorTerm {
                        match_label_expressions: vec![TopologySelectorLabelRequirement {
                            key: "topology.tugboat.cloud/zone".to_string(),
                            values: vec!["zone-a".to_string()],
                        }],
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }],
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
            VolumeTopologyFilter.filter(&ctx, &node),
            FilterResult::Reject(reason) if reason.contains("fast")
        ));
    }
}
