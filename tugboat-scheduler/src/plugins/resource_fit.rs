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

/// Filter plugin: reject nodes that don't have enough resources for the Ship.
pub struct ResourceFitFilter;

impl ResourceFitFilter {
    /// Calculate allocatable resources for a node: base_resource * overcommit_ratio.
    /// Returns (allocatable_cpu, allocatable_memory_bytes).
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

impl FilterPlugin for ResourceFitFilter {
    fn name(&self) -> &str {
        "ResourceFit"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let node_name = node
            .object_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            .unwrap_or("unknown");

        let (alloc_cpu, alloc_memory) = Self::node_allocatable(node);
        let (used_cpu, used_memory) = ctx.node_resource_usage(node_name);
        let (req_cpu, req_memory) = ctx.requested_resources();

        let avail_cpu = alloc_cpu.saturating_sub(used_cpu);
        let avail_memory = alloc_memory.saturating_sub(used_memory);

        if req_cpu > avail_cpu {
            return FilterResult::Reject(format!(
                "insufficient CPU: requested={req_cpu}, available={avail_cpu}"
            ));
        }

        if req_memory > avail_memory {
            return FilterResult::Reject(format!(
                "insufficient memory: requested={req_memory}, available={avail_memory}"
            ));
        }

        FilterResult::Accept
    }
}
