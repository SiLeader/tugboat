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
use std::collections::BTreeSet;
use tugboat_resources::manifests::core::v1::{
    NetworkClassSpec, Node, NodeCniPluginStatus, ShipNetworkClassReference,
};

/// Filter plugin: reject nodes that do not advertise the CNI plugins required
/// by the Ship's requested NetworkClass / ClusterNetworkClass references.
pub struct NetworkFitFilter;

impl FilterPlugin for NetworkFitFilter {
    fn name(&self) -> &str {
        "NetworkFit"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let Some(status) = node.status.as_ref() else {
            return FilterResult::Reject("node has no published network status".to_string());
        };

        let mut required_plugins = BTreeSet::from(["loopback".to_string()]);
        let network_refs = ctx
            .ship
            .spec
            .as_ref()
            .map(|spec| spec.network_class_ref.as_slice())
            .unwrap_or(&[]);

        for network_ref in network_refs {
            let spec = match resolve_network_class_spec(ctx, network_ref) {
                Ok(spec) => spec,
                Err(reason) => return FilterResult::Reject(reason),
            };
            if let Err(reason) = collect_required_plugins(&mut required_plugins, spec, network_ref)
            {
                return FilterResult::Reject(reason);
            }
        }

        for plugin in required_plugins {
            if let Err(reason) = require_plugin_ready(&status.cni_plugins, &plugin) {
                return FilterResult::Reject(reason);
            }
        }

        FilterResult::Accept
    }
}

fn resolve_network_class_spec<'a>(
    ctx: &'a SchedulingContext,
    network_ref: &ShipNetworkClassReference,
) -> Result<&'a NetworkClassSpec, String> {
    if network_ref.api_group != "core" && !network_ref.api_group.is_empty() {
        return Err(format!(
            "network reference '{}' uses unsupported apiGroup '{}'",
            network_class_ref_name(ctx, network_ref),
            network_ref.api_group
        ));
    }

    match network_ref.kind.as_str() {
        "ClusterNetworkClass" => ctx
            .find_cluster_network_class(&network_ref.name)
            .and_then(|network_class| network_class.spec.as_ref())
            .ok_or_else(|| format!("cluster network class '{}' was not found", network_ref.name)),
        "NetworkClass" => ctx
            .find_network_class(ctx.ship_namespace(), &network_ref.name)
            .and_then(|network_class| network_class.spec.as_ref())
            .ok_or_else(|| {
                format!(
                    "network class '{}/{}' was not found",
                    ctx.ship_namespace(),
                    network_ref.name
                )
            }),
        _ => Err(format!(
            "network reference '{}' uses unsupported kind '{}'",
            network_class_ref_name(ctx, network_ref),
            network_ref.kind
        )),
    }
}

fn collect_required_plugins(
    required_plugins: &mut BTreeSet<String>,
    spec: &NetworkClassSpec,
    network_ref: &ShipNetworkClassReference,
) -> Result<(), String> {
    let plugin = normalized_plugin(spec);
    match plugin {
        "bridge" => {
            required_plugins.insert("bridge".to_string());
        }
        "flannel" => {
            required_plugins.insert("bridge".to_string());
            required_plugins.insert("flannel".to_string());
            if spec
                .flannel
                .as_ref()
                .and_then(|flannel| flannel.port_mappings)
                .unwrap_or(false)
            {
                required_plugins.insert("portmap".to_string());
            }
        }
        other => {
            return Err(format!(
                "network reference '{}' requests unsupported cniPlugin '{}'",
                network_ref.name, other
            ));
        }
    }

    Ok(())
}

fn require_plugin_ready(statuses: &[NodeCniPluginStatus], plugin: &str) -> Result<(), String> {
    let Some(status) = statuses.iter().find(|status| status.name == plugin) else {
        return Err(format!(
            "node does not advertise required CNI plugin '{}'",
            plugin
        ));
    };

    if status.ready.unwrap_or(false) {
        Ok(())
    } else if status.message.is_empty() {
        Err(format!("required CNI plugin '{}' is not ready", plugin))
    } else {
        Err(format!(
            "required CNI plugin '{}' is not ready: {}",
            plugin, status.message
        ))
    }
}

fn normalized_plugin(spec: &NetworkClassSpec) -> &str {
    let plugin = spec.cni_plugin.trim();
    if plugin.is_empty() { "bridge" } else { plugin }
}

fn network_class_ref_name(
    ctx: &SchedulingContext,
    network_ref: &ShipNetworkClassReference,
) -> String {
    match network_ref.kind.as_str() {
        "NetworkClass" => format!("{}/{}", ctx.ship_namespace(), network_ref.name),
        _ => network_ref.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use tugboat_resources::manifests::core::v1::{
        ClusterNetworkClass, FlannelNetworkClass, NetworkClass, NodeStatus, Ship, ShipClass,
        ShipSpec,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn accepts_ship_without_network_classes_when_loopback_is_ready() {
        let filter = NetworkFitFilter;
        let ctx = scheduling_context(vec![], vec![], ship(vec![]));
        let node = node_with_plugins(&[("loopback", true, "ready")]);

        assert!(matches!(filter.filter(&ctx, &node), FilterResult::Accept));
    }

    #[test]
    fn rejects_node_missing_bridge_for_default_network_class() {
        let filter = NetworkFitFilter;
        let ctx = scheduling_context(
            vec![network_class("default", "frontend", "")],
            vec![],
            ship(vec![network_ref("NetworkClass", "frontend")]),
        );
        let node = node_with_plugins(&[("loopback", true, "ready")]);

        match filter.filter(&ctx, &node) {
            FilterResult::Reject(reason) => assert!(reason.contains("bridge")),
            FilterResult::Accept => panic!("expected node to be rejected"),
        }
    }

    #[test]
    fn rejects_flannel_network_when_portmap_is_not_ready() {
        let filter = NetworkFitFilter;
        let ctx = scheduling_context(
            vec![],
            vec![cluster_network_class("overlay", true)],
            ship(vec![network_ref("ClusterNetworkClass", "overlay")]),
        );
        let node = node_with_plugins(&[
            ("loopback", true, "ready"),
            ("bridge", true, "ready"),
            ("flannel", true, "ready"),
            ("portmap", false, "missing"),
        ]);

        match filter.filter(&ctx, &node) {
            FilterResult::Reject(reason) => assert!(reason.contains("portmap")),
            FilterResult::Accept => panic!("expected node to be rejected"),
        }
    }

    fn scheduling_context(
        network_classes: Vec<NetworkClass>,
        cluster_network_classes: Vec<ClusterNetworkClass>,
        ship: Ship,
    ) -> SchedulingContext {
        SchedulingContext {
            ship,
            ship_class: ShipClass::default(),
            all_cluster_network_classes: cluster_network_classes,
            all_network_classes: network_classes,
            all_runtime_classes: Vec::new(),
            all_ships: Vec::new(),
            all_ship_classes: Vec::new(),
            all_persistent_volume_claims: Vec::new(),
            all_persistent_volumes: Vec::new(),
        }
    }

    fn ship(network_class_ref: Vec<ShipNetworkClassReference>) -> Ship {
        Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-a".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                ship_class: "small".to_string(),
                network_class_ref,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn network_ref(kind: &str, name: &str) -> ShipNetworkClassReference {
        ShipNetworkClassReference {
            kind: kind.to_string(),
            name: name.to_string(),
            api_group: "core".to_string(),
        }
    }

    fn network_class(namespace: &str, name: &str, cni_plugin: &str) -> NetworkClass {
        NetworkClass {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            }),
            spec: Some(NetworkClassSpec {
                cni_plugin: cni_plugin.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn cluster_network_class(name: &str, port_mappings: bool) -> ClusterNetworkClass {
        ClusterNetworkClass {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            spec: Some(NetworkClassSpec {
                cni_plugin: "flannel".to_string(),
                flannel: Some(FlannelNetworkClass {
                    port_mappings: Some(port_mappings),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn node_with_plugins(plugins: &[(&str, bool, &str)]) -> Node {
        Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-a".to_string()),
                ..Default::default()
            }),
            status: Some(NodeStatus {
                cni_plugins: plugins
                    .iter()
                    .map(|(name, ready, message)| NodeCniPluginStatus {
                        name: (*name).to_string(),
                        ready: Some(*ready),
                        message: (*message).to_string(),
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
