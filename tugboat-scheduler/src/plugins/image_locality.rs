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

use crate::framework::{SchedulingContext, ScorePlugin, ScoreResult};
use tugboat_resources::manifests::core::v1::Node;

pub struct ImageLocalityScorer;

impl ScorePlugin for ImageLocalityScorer {
    fn name(&self) -> &str {
        "ImageLocality"
    }

    fn score(&self, ctx: &SchedulingContext, node: &Node) -> ScoreResult {
        let Some(image) = ctx
            .ship
            .spec
            .as_ref()
            .map(|spec| spec.image.as_str())
            .filter(|image| !image.is_empty())
        else {
            return ScoreResult::Skip;
        };

        let cached = node
            .status
            .as_ref()
            .is_some_and(|status| status.images.iter().any(|item| item.image == image));
        ScoreResult::Score(if cached { 100 } else { 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use tugboat_resources::manifests::core::v1::{
        NodeImageStatus, NodeStatus, Ship, ShipClass, ShipSpec,
    };

    #[test]
    fn scores_cached_image_higher() {
        let ctx = SchedulingContext {
            ship: Ship {
                spec: Some(ShipSpec {
                    image: "registry.example/app:v1".to_string(),
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
            status: Some(NodeStatus {
                images: vec![NodeImageStatus {
                    image: "registry.example/app:v1".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(matches!(
            ImageLocalityScorer.score(&ctx, &node),
            ScoreResult::Score(100)
        ));
    }
}
