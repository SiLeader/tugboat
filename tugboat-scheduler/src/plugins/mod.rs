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

mod least_allocated;
mod network_fit;
mod resource_fit;
mod storage_fit;
mod taint_toleration;

use crate::framework::{FilterPlugin, ScorePlugin};

pub fn create_filter_plugin(name: &str) -> Option<Box<dyn FilterPlugin>> {
    match name {
        "NetworkFit" => Some(Box::new(network_fit::NetworkFitFilter)),
        "TaintToleration" => Some(Box::new(taint_toleration::TaintTolerationFilter)),
        "ResourceFit" => Some(Box::new(resource_fit::ResourceFitFilter)),
        "StorageFit" => Some(Box::new(storage_fit::StorageFitFilter)),
        _ => None,
    }
}

pub fn create_score_plugin(name: &str) -> Option<Box<dyn ScorePlugin>> {
    match name {
        "TaintToleration" => Some(Box::new(taint_toleration::TaintTolerationScorer)),
        "LeastAllocated" => Some(Box::new(least_allocated::LeastAllocatedScorer)),
        _ => None,
    }
}
