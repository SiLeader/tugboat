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

mod plugin;
mod types;

pub use plugin::{FilterPlugin, ScorePlugin};
pub use types::{FilterResult, SchedulingContext, ScoreResult};

use tugboat_resources::manifests::core::v1::Node;

pub struct Framework {
    filter_plugins: Vec<Box<dyn FilterPlugin>>,
    score_plugins: Vec<Box<dyn ScorePlugin>>,
}

impl Default for Framework {
    fn default() -> Self {
        Self::new()
    }
}

impl Framework {
    pub fn new() -> Self {
        Self {
            filter_plugins: Vec::new(),
            score_plugins: Vec::new(),
        }
    }

    pub fn add_filter_plugin(&mut self, plugin: Box<dyn FilterPlugin>) {
        tracing::info!("Registered filter plugin: {}", plugin.name());
        self.filter_plugins.push(plugin);
    }

    pub fn add_score_plugin(&mut self, plugin: Box<dyn ScorePlugin>) {
        tracing::info!("Registered score plugin: {}", plugin.name());
        self.score_plugins.push(plugin);
    }

    /// Run all filter plugins. Returns nodes that pass all filters.
    pub fn filter(&self, ctx: &SchedulingContext, nodes: &[Node]) -> Vec<Node> {
        nodes
            .iter()
            .filter(|node| {
                for plugin in &self.filter_plugins {
                    match plugin.filter(ctx, node) {
                        FilterResult::Accept => {}
                        FilterResult::Reject(reason) => {
                            let node_name = node
                                .object_meta
                                .as_ref()
                                .and_then(|m| m.name.as_deref())
                                .unwrap_or("unknown");
                            tracing::debug!(
                                "Node {node_name} rejected by {}: {reason}",
                                plugin.name()
                            );
                            return false;
                        }
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    /// Run all score plugins and return nodes sorted by total score (highest first).
    pub fn score(&self, ctx: &SchedulingContext, nodes: &[Node]) -> Vec<(Node, i64)> {
        let mut scored: Vec<(Node, i64)> = nodes
            .iter()
            .map(|node| {
                let total_score: i64 = self
                    .score_plugins
                    .iter()
                    .map(|plugin| match plugin.score(ctx, node) {
                        ScoreResult::Score(s) => s,
                        ScoreResult::Skip => 0,
                    })
                    .sum();
                (node.clone(), total_score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored
    }

    /// Run the full scheduling pipeline: filter then score.
    /// Returns the best node, or None if no node passes all filters.
    pub fn schedule(&self, ctx: &SchedulingContext, nodes: &[Node]) -> Option<Node> {
        let filtered = self.filter(ctx, nodes);
        if filtered.is_empty() {
            return None;
        }
        let scored = self.score(ctx, &filtered);
        scored.into_iter().next().map(|(node, _)| node)
    }
}
