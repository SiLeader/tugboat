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

mod image_locality;
mod least_allocated;
mod network_fit;
mod node_affinity;
mod node_affinity_score;
mod resource_fit;
mod runtime_class_fit;
mod selectors;
mod ship_affinity;
mod storage_binding_ready;
mod storage_fit;
mod taint_toleration;
mod topology_spread;
mod unschedulable;
mod volume_topology;

use crate::framework::{FilterPlugin, ScorePlugin};

pub fn create_filter_plugin(name: &str) -> Option<Box<dyn FilterPlugin>> {
    match name {
        "Unschedulable" => Some(Box::new(unschedulable::UnschedulableFilter)),
        "NetworkFit" => Some(Box::new(network_fit::NetworkFitFilter)),
        "TaintToleration" => Some(Box::new(taint_toleration::TaintTolerationFilter)),
        "ResourceFit" => Some(Box::new(resource_fit::ResourceFitFilter)),
        "RuntimeClassFit" => Some(Box::new(runtime_class_fit::RuntimeClassFitFilter)),
        "StorageFit" => Some(Box::new(storage_fit::StorageFitFilter)),
        "StorageBindingReady" => Some(Box::new(storage_binding_ready::StorageBindingReadyFilter)),
        "VolumeTopology" => Some(Box::new(volume_topology::VolumeTopologyFilter)),
        "NodeAffinity" => Some(Box::new(node_affinity::NodeAffinityFilter)),
        "ShipAffinity" => Some(Box::new(ship_affinity::ShipAffinityFilter)),
        "TopologySpread" => Some(Box::new(topology_spread::TopologySpreadFilter)),
        _ => None,
    }
}

pub fn create_score_plugin(name: &str) -> Option<Box<dyn ScorePlugin>> {
    match name {
        "TaintToleration" => Some(Box::new(taint_toleration::TaintTolerationScorer)),
        "LeastAllocated" => Some(Box::new(least_allocated::LeastAllocatedScorer)),
        "NodeAffinity" => Some(Box::new(node_affinity_score::NodeAffinityScorer)),
        "TopologySpread" => Some(Box::new(topology_spread::TopologySpreadScorer)),
        "ImageLocality" => Some(Box::new(image_locality::ImageLocalityScorer)),
        _ => None,
    }
}
