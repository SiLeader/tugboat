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
use tugboat_resources::NODE_RUNTIME_CLASS_LABEL_KEY;
use tugboat_resources::manifests::core::v1::Node;

/// Rejects nodes whose advertised RuntimeClass cannot satisfy the Ship's
/// runtime requirements.
pub struct RuntimeClassFitFilter;

impl FilterPlugin for RuntimeClassFitFilter {
    fn name(&self) -> &str {
        "RuntimeClassFit"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let requested_runtime_class = ctx
            .ship
            .spec
            .as_ref()
            .and_then(|spec| spec.runtime_class.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requires_live_migration = ship_requires_live_migration(ctx);

        if requested_runtime_class.is_none() && !requires_live_migration {
            return FilterResult::Accept;
        }

        let Some(node_runtime_class_name) = node_runtime_class_name(node) else {
            return FilterResult::Reject(format!(
                "node does not declare '{}' label",
                NODE_RUNTIME_CLASS_LABEL_KEY
            ));
        };

        if let Some(requested_runtime_class) = requested_runtime_class
            && node_runtime_class_name != requested_runtime_class
        {
            return FilterResult::Reject(format!(
                "node runtime class '{}' does not match requested runtime class '{}'",
                node_runtime_class_name, requested_runtime_class
            ));
        }

        let Some(runtime_class) = ctx.find_runtime_class(node_runtime_class_name) else {
            return FilterResult::Reject(format!(
                "runtime class '{}' referenced by node was not found",
                node_runtime_class_name
            ));
        };

        if requires_live_migration
            && !runtime_class
                .spec
                .as_ref()
                .map(|spec| spec.live_migration)
                .unwrap_or(false)
        {
            return FilterResult::Reject(format!(
                "runtime class '{}' does not support live migration",
                node_runtime_class_name
            ));
        }

        FilterResult::Accept
    }
}

fn node_runtime_class_name(node: &Node) -> Option<&str> {
    node.object_meta
        .as_ref()
        .and_then(|meta| meta.labels.get(NODE_RUNTIME_CLASS_LABEL_KEY))
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn ship_requires_live_migration(ctx: &SchedulingContext) -> bool {
    ctx.ship
        .spec
        .as_ref()
        .and_then(|spec| spec.target_node_name.as_deref())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::{
        RuntimeClass, RuntimeClassSpec, Ship, ShipClass, ShipSpec,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn accepts_ship_without_runtime_class() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(ship(None, None), vec![runtime_class("kata", true)]);
        let node = node("node-a", Some("kata"));

        assert!(matches!(filter.filter(&ctx, &node), FilterResult::Accept));
    }

    #[test]
    fn accepts_migrating_ship_when_runtime_class_supports_live_migration() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(
            ship(Some("kata"), Some("node-b")),
            vec![runtime_class("kata", true)],
        );
        let node = node("node-a", Some("kata"));

        assert!(matches!(filter.filter(&ctx, &node), FilterResult::Accept));
    }

    #[test]
    fn rejects_migrating_ship_when_runtime_class_disables_live_migration() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(
            ship(Some("kata"), Some("node-b")),
            vec![runtime_class("kata", false)],
        );
        let node = node("node-a", Some("kata"));

        assert!(matches!(
            filter.filter(&ctx, &node),
            FilterResult::Reject(_)
        ));
    }

    #[test]
    fn accepts_migrating_ship_without_requested_runtime_class_when_node_supports_live_migration() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(
            ship(None, Some("node-b")),
            vec![runtime_class("kata", true)],
        );
        let node = node("node-a", Some("kata"));

        assert!(matches!(filter.filter(&ctx, &node), FilterResult::Accept));
    }

    #[test]
    fn rejects_migrating_ship_without_requested_runtime_class_when_node_disables_live_migration() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(
            ship(None, Some("node-b")),
            vec![runtime_class("kata", false)],
        );
        let node = node("node-a", Some("kata"));

        assert!(matches!(
            filter.filter(&ctx, &node),
            FilterResult::Reject(_)
        ));
    }

    #[test]
    fn rejects_when_node_runtime_class_does_not_exist() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(ship(Some("kata"), None), Vec::new());
        let node = node("node-a", Some("kata"));

        assert!(matches!(
            filter.filter(&ctx, &node),
            FilterResult::Reject(_)
        ));
    }

    #[test]
    fn accepts_regular_ship_when_runtime_class_matches() {
        let filter = RuntimeClassFitFilter;
        let ctx = scheduling_context(ship(Some("kata"), None), vec![runtime_class("kata", false)]);
        let node = node("node-a", Some("kata"));

        assert!(matches!(filter.filter(&ctx, &node), FilterResult::Accept));
    }

    fn scheduling_context(ship: Ship, runtime_classes: Vec<RuntimeClass>) -> SchedulingContext {
        SchedulingContext {
            ship,
            ship_class: ShipClass::default(),
            all_cluster_network_classes: Vec::new(),
            all_network_classes: Vec::new(),
            all_runtime_classes: runtime_classes,
            all_ships: Vec::new(),
            all_nodes: Vec::new(),
            all_ship_classes: Vec::new(),
            all_persistent_volume_claims: Vec::new(),
            all_persistent_volumes: Vec::new(),
            all_storage_classes: Vec::new(),
        }
    }

    fn ship(runtime_class: Option<&str>, target_node_name: Option<&str>) -> Ship {
        Ship {
            object_meta: Some(ObjectMeta {
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                ship_class: "small".to_string(),
                runtime_class: runtime_class.map(str::to_string),
                target_node_name: target_node_name.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn runtime_class(name: &str, live_migration: bool) -> RuntimeClass {
        RuntimeClass {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            spec: Some(RuntimeClassSpec {
                live_migration,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn node(name: &str, runtime_class_name: Option<&str>) -> Node {
        let labels = runtime_class_name.map(|value| {
            HashMap::from([(NODE_RUNTIME_CLASS_LABEL_KEY.to_string(), value.to_string())])
        });

        Node {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                labels: labels.unwrap_or_default(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
