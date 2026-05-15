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
use crate::plugins::selectors::node_selector_term_matches;
use tugboat_resources::manifests::core::v1::Node;

pub struct NodeAffinityScorer;

impl ScorePlugin for NodeAffinityScorer {
    fn name(&self) -> &str {
        "NodeAffinity"
    }

    fn score(&self, ctx: &SchedulingContext, node: &Node) -> ScoreResult {
        let Some(node_affinity) = ctx
            .ship
            .spec
            .as_ref()
            .and_then(|spec| spec.affinity.as_ref())
            .and_then(|affinity| affinity.node_affinity.as_ref())
        else {
            return ScoreResult::Skip;
        };

        if node_affinity.preferred_during_scheduling.is_empty() {
            return ScoreResult::Skip;
        }

        let labels = node
            .object_meta
            .as_ref()
            .map(|meta| &meta.labels)
            .cloned()
            .unwrap_or_default();
        let mut max_weight = 0i64;
        let mut matched_weight = 0i64;
        for term in &node_affinity.preferred_during_scheduling {
            let weight = term.weight.clamp(0, 100) as i64;
            max_weight += weight;
            if term
                .preference
                .as_ref()
                .is_some_and(|preference| node_selector_term_matches(preference, &labels))
            {
                matched_weight += weight;
            }
        }

        if max_weight == 0 {
            return ScoreResult::Skip;
        }

        ScoreResult::Score(matched_weight * 100 / max_weight)
    }
}
