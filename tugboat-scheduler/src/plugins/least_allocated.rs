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

/// Score plugin: prefer nodes with the most available resources.
pub struct LeastAllocatedScorer;

impl LeastAllocatedScorer {
    fn node_allocatable(node: &Node) -> (u64, u64) {
        let spec = node.spec.as_ref();
        let resource = spec.and_then(|s| s.resource.as_ref());
        let overcommit = spec.and_then(|s| s.overcommit.as_ref());

        let base_cpu = resource.map(|r| r.cpu).unwrap_or(0);
        let base_memory = resource.map(|r| r.memory).unwrap_or(0);

        let cpu_ratio: f64 = overcommit
            .and_then(|o| o.cpu_ratio.parse().ok())
            .unwrap_or(1.0);
        let memory_ratio: f64 = overcommit
            .and_then(|o| o.memory_ratio.parse().ok())
            .unwrap_or(1.0);

        let alloc_cpu = (base_cpu as f64 * cpu_ratio) as u64;
        let alloc_memory = (base_memory as f64 * memory_ratio) as u64;

        (alloc_cpu, alloc_memory)
    }
}

impl ScorePlugin for LeastAllocatedScorer {
    fn name(&self) -> &str {
        "LeastAllocated"
    }

    fn score(&self, ctx: &SchedulingContext, node: &Node) -> ScoreResult {
        let node_name = node
            .object_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            .unwrap_or("unknown");

        let (alloc_cpu, alloc_memory) = Self::node_allocatable(node);
        let (used_cpu, used_memory) = ctx.node_resource_usage(node_name);

        // Calculate the fraction of available resources (0-100 scale).
        let cpu_score = if alloc_cpu > 0 {
            ((alloc_cpu.saturating_sub(used_cpu)) as f64 / alloc_cpu as f64 * 100.0) as i64
        } else {
            0
        };

        let memory_score = if alloc_memory > 0 {
            ((alloc_memory.saturating_sub(used_memory)) as f64 / alloc_memory as f64 * 100.0) as i64
        } else {
            0
        };

        // Average of CPU and memory availability scores.
        ScoreResult::Score((cpu_score + memory_score) / 2)
    }
}
