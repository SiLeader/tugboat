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

use crate::framework::{FilterPlugin, FilterResult, SchedulingContext, ScorePlugin, ScoreResult};
use tugboat_resources::manifests::core::v1::{Node, Taint, Toleration};

/// Checks if a toleration matches a taint.
fn toleration_matches_taint(toleration: &Toleration, taint: &Taint) -> bool {
    // If toleration has no key, it matches all taints with the same effect
    let Some(ref tol_key) = toleration.key else {
        return toleration.effect.is_empty() || toleration.effect == taint.effect;
    };

    if tol_key != &taint.key {
        return false;
    }

    if !toleration.effect.is_empty() && toleration.effect != taint.effect {
        return false;
    }

    match toleration.operator.as_str() {
        "Exists" => true,
        "Equal" | "" => toleration.value.as_deref().unwrap_or("") == taint.value,
        _ => false,
    }
}

fn ship_tolerates_taint(tolerations: &[Toleration], taint: &Taint) -> bool {
    tolerations
        .iter()
        .any(|t| toleration_matches_taint(t, taint))
}

/// Filter plugin: reject nodes with NoSchedule taints not tolerated by the Ship.
pub struct TaintTolerationFilter;

impl FilterPlugin for TaintTolerationFilter {
    fn name(&self) -> &str {
        "TaintToleration"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let taints = node
            .spec
            .as_ref()
            .map(|s| s.taints.as_slice())
            .unwrap_or(&[]);

        let tolerations = ctx
            .ship
            .spec
            .as_ref()
            .map(|s| s.tolerations.as_slice())
            .unwrap_or(&[]);

        for taint in taints {
            if taint.effect == "NoSchedule" && !ship_tolerates_taint(tolerations, taint) {
                return FilterResult::Reject(format!(
                    "node has NoSchedule taint {}={} not tolerated",
                    taint.key, taint.value
                ));
            }
        }

        FilterResult::Accept
    }
}

/// Score plugin: penalize nodes with PreferNoSchedule taints not tolerated by the Ship.
pub struct TaintTolerationScorer;

impl ScorePlugin for TaintTolerationScorer {
    fn name(&self) -> &str {
        "TaintToleration"
    }

    fn score(&self, ctx: &SchedulingContext, node: &Node) -> ScoreResult {
        let taints = node
            .spec
            .as_ref()
            .map(|s| s.taints.as_slice())
            .unwrap_or(&[]);

        let tolerations = ctx
            .ship
            .spec
            .as_ref()
            .map(|s| s.tolerations.as_slice())
            .unwrap_or(&[]);

        let mut penalty: i64 = 0;
        for taint in taints {
            if taint.effect == "PreferNoSchedule" && !ship_tolerates_taint(tolerations, taint) {
                penalty -= 50;
            }
        }

        ScoreResult::Score(penalty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_taint(key: &str, value: &str, effect: &str) -> Taint {
        Taint {
            key: key.to_string(),
            value: value.to_string(),
            effect: effect.to_string(),
        }
    }

    fn make_toleration(
        key: Option<&str>,
        operator: &str,
        value: Option<&str>,
        effect: &str,
    ) -> Toleration {
        Toleration {
            key: key.map(|k| k.to_string()),
            operator: operator.to_string(),
            value: value.map(|v| v.to_string()),
            effect: effect.to_string(),
            toleration_seconds: None,
        }
    }

    #[test]
    fn test_equal_operator_matches() {
        let taint = make_taint("gpu", "true", "NoSchedule");
        let toleration = make_toleration(Some("gpu"), "Equal", Some("true"), "NoSchedule");
        assert!(toleration_matches_taint(&toleration, &taint));
    }

    #[test]
    fn test_equal_operator_mismatch_value() {
        let taint = make_taint("gpu", "true", "NoSchedule");
        let toleration = make_toleration(Some("gpu"), "Equal", Some("false"), "NoSchedule");
        assert!(!toleration_matches_taint(&toleration, &taint));
    }

    #[test]
    fn test_exists_operator_matches() {
        let taint = make_taint("gpu", "true", "NoSchedule");
        let toleration = make_toleration(Some("gpu"), "Exists", None, "NoSchedule");
        assert!(toleration_matches_taint(&toleration, &taint));
    }

    #[test]
    fn test_no_key_tolerates_all() {
        let taint = make_taint("anything", "value", "NoSchedule");
        let toleration = make_toleration(None, "Exists", None, "");
        assert!(toleration_matches_taint(&toleration, &taint));
    }

    #[test]
    fn test_effect_mismatch() {
        let taint = make_taint("gpu", "true", "NoSchedule");
        let toleration = make_toleration(Some("gpu"), "Equal", Some("true"), "NoExecute");
        assert!(!toleration_matches_taint(&toleration, &taint));
    }
}
