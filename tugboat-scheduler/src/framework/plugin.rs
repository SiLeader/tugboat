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

use super::types::{FilterResult, SchedulingContext, ScoreResult};
use tugboat_resources::manifests::core::v1::Node;

/// A plugin that determines whether a node is eligible for scheduling.
pub trait FilterPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult;
}

/// A plugin that scores a node's suitability for scheduling.
pub trait ScorePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn score(&self, ctx: &SchedulingContext, node: &Node) -> ScoreResult;
}
