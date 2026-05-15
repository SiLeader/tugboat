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

use crate::manifests::core::v1::{
    NodeAffinity, NodeSelector, PreferredSchedulingTerm, Ship, ShipAffinity, ShipAffinityTerm,
    ShipAntiAffinity, TopologySpreadConstraint, WeightedShipAffinityTerm,
};
use crate::manifests::meta::v1::{LabelSelector, LabelSelectorRequirement};
use crate::validators::Validator;

const VALID_NODE_SELECTOR_OPERATORS: &[&str] =
    &["In", "NotIn", "Exists", "DoesNotExist", "Gt", "Lt"];
const VALID_LABEL_SELECTOR_OPERATORS: &[&str] = &["In", "NotIn", "Exists", "DoesNotExist"];
const VALID_WHEN_UNSATISFIABLE: &[&str] = &["DoNotSchedule", "ScheduleAnyway"];

pub struct ShipSchedulingValidator;

impl Validator<Ship> for ShipSchedulingValidator {
    fn validate(&self, ship: &Ship) -> bool {
        let Some(spec) = ship.spec.as_ref() else {
            return true;
        };

        let affinity_valid = spec.affinity.as_ref().is_none_or(|affinity| {
            affinity
                .node_affinity
                .as_ref()
                .is_none_or(valid_node_affinity)
                && affinity
                    .ship_affinity
                    .as_ref()
                    .is_none_or(valid_ship_affinity)
                && affinity
                    .ship_anti_affinity
                    .as_ref()
                    .is_none_or(valid_ship_anti_affinity)
        });

        affinity_valid
            && spec
                .topology_spread_constraints
                .iter()
                .all(valid_topology_spread_constraint)
    }
}

fn valid_node_affinity(affinity: &NodeAffinity) -> bool {
    affinity
        .required_during_scheduling
        .as_ref()
        .is_none_or(valid_node_selector)
        && affinity
            .preferred_during_scheduling
            .iter()
            .all(valid_preferred_scheduling_term)
}

fn valid_node_selector(selector: &NodeSelector) -> bool {
    selector.node_selector_terms.iter().all(|term| {
        term.match_expressions
            .iter()
            .chain(term.match_fields.iter())
            .all(|requirement| {
                !requirement.key.is_empty()
                    && VALID_NODE_SELECTOR_OPERATORS.contains(&requirement.operator.as_str())
            })
    })
}

fn valid_preferred_scheduling_term(term: &PreferredSchedulingTerm) -> bool {
    (1..=100).contains(&term.weight)
        && term.preference.as_ref().is_some_and(|preference| {
            preference
                .match_expressions
                .iter()
                .chain(preference.match_fields.iter())
                .all(|requirement| {
                    !requirement.key.is_empty()
                        && VALID_NODE_SELECTOR_OPERATORS.contains(&requirement.operator.as_str())
                })
        })
}

fn valid_ship_affinity(affinity: &ShipAffinity) -> bool {
    affinity
        .required_during_scheduling
        .iter()
        .all(valid_ship_affinity_term)
        && affinity
            .preferred_during_scheduling
            .iter()
            .all(valid_weighted_ship_affinity_term)
}

fn valid_ship_anti_affinity(affinity: &ShipAntiAffinity) -> bool {
    affinity
        .required_during_scheduling
        .iter()
        .all(valid_ship_affinity_term)
        && affinity
            .preferred_during_scheduling
            .iter()
            .all(valid_weighted_ship_affinity_term)
}

fn valid_weighted_ship_affinity_term(term: &WeightedShipAffinityTerm) -> bool {
    (1..=100).contains(&term.weight)
        && term
            .ship_affinity_term
            .as_ref()
            .is_some_and(valid_ship_affinity_term)
}

fn valid_ship_affinity_term(term: &ShipAffinityTerm) -> bool {
    !term.topology_key.is_empty()
        && term
            .label_selector
            .as_ref()
            .is_none_or(valid_label_selector)
}

fn valid_topology_spread_constraint(constraint: &TopologySpreadConstraint) -> bool {
    constraint.max_skew >= 1
        && !constraint.topology_key.is_empty()
        && VALID_WHEN_UNSATISFIABLE.contains(&constraint.when_unsatisfiable.as_str())
        && constraint
            .label_selector
            .as_ref()
            .is_none_or(valid_label_selector)
}

fn valid_label_selector(selector: &LabelSelector) -> bool {
    selector
        .match_expressions
        .iter()
        .all(valid_label_selector_requirement)
}

fn valid_label_selector_requirement(requirement: &LabelSelectorRequirement) -> bool {
    !requirement.key.is_empty()
        && VALID_LABEL_SELECTOR_OPERATORS.contains(&requirement.operator.as_str())
}
