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

use std::collections::HashMap;
use tugboat_resources::manifests::core::v1::{
    NodeSelector, NodeSelectorRequirement, NodeSelectorTerm, TopologySelectorTerm,
};
use tugboat_resources::manifests::meta::v1::{LabelSelector, LabelSelectorRequirement};

pub(crate) fn node_selector_matches(
    selector: &NodeSelector,
    labels: &HashMap<String, String>,
) -> bool {
    !selector.node_selector_terms.is_empty()
        && selector
            .node_selector_terms
            .iter()
            .any(|term| node_selector_term_matches(term, labels))
}

pub(crate) fn node_selector_term_matches(
    term: &NodeSelectorTerm,
    labels: &HashMap<String, String>,
) -> bool {
    term.match_expressions
        .iter()
        .chain(term.match_fields.iter())
        .all(|requirement| node_selector_requirement_matches(requirement, labels))
}

pub(crate) fn first_unsatisfied_node_selector_key(
    selector: &NodeSelector,
    labels: &HashMap<String, String>,
) -> Option<String> {
    if node_selector_matches(selector, labels) {
        return None;
    }
    selector
        .node_selector_terms
        .iter()
        .flat_map(|term| {
            term.match_expressions
                .iter()
                .chain(term.match_fields.iter())
        })
        .find(|requirement| !node_selector_requirement_matches(requirement, labels))
        .map(|requirement| requirement.key.clone())
}

fn node_selector_requirement_matches(
    requirement: &NodeSelectorRequirement,
    labels: &HashMap<String, String>,
) -> bool {
    let value = labels.get(&requirement.key).map(String::as_str);
    match requirement.operator.as_str() {
        "In" => value.is_some_and(|value| requirement.values.iter().any(|item| item == value)),
        "NotIn" => value.is_none_or(|value| !requirement.values.iter().any(|item| item == value)),
        "Exists" => value.is_some(),
        "DoesNotExist" => value.is_none(),
        "Gt" => compare_i64(
            value,
            requirement.values.first().map(String::as_str),
            |a, b| a > b,
        ),
        "Lt" => compare_i64(
            value,
            requirement.values.first().map(String::as_str),
            |a, b| a < b,
        ),
        _ => false,
    }
}

fn compare_i64(
    value: Option<&str>,
    threshold: Option<&str>,
    f: impl FnOnce(i64, i64) -> bool,
) -> bool {
    let (Some(value), Some(threshold)) = (value, threshold) else {
        return false;
    };
    let (Ok(value), Ok(threshold)) = (value.parse::<i64>(), threshold.parse::<i64>()) else {
        return false;
    };
    f(value, threshold)
}

pub(crate) fn label_selector_matches(
    selector: Option<&LabelSelector>,
    labels: &HashMap<String, String>,
) -> bool {
    let Some(selector) = selector else {
        return true;
    };
    selector
        .match_labels
        .iter()
        .all(|(key, value)| labels.get(key) == Some(value))
        && selector
            .match_expressions
            .iter()
            .all(|requirement| label_selector_requirement_matches(requirement, labels))
}

fn label_selector_requirement_matches(
    requirement: &LabelSelectorRequirement,
    labels: &HashMap<String, String>,
) -> bool {
    let value = labels.get(&requirement.key).map(String::as_str);
    match requirement.operator.as_str() {
        "In" => value.is_some_and(|value| requirement.values.iter().any(|item| item == value)),
        "NotIn" => value.is_none_or(|value| !requirement.values.iter().any(|item| item == value)),
        "Exists" => value.is_some(),
        "DoesNotExist" => value.is_none(),
        _ => false,
    }
}

pub(crate) fn topology_selector_terms_match(
    terms: &[TopologySelectorTerm],
    labels: &HashMap<String, String>,
) -> bool {
    terms.is_empty()
        || terms
            .iter()
            .any(|term| topology_selector_term_matches(term, labels))
}

fn topology_selector_term_matches(
    term: &TopologySelectorTerm,
    labels: &HashMap<String, String>,
) -> bool {
    term.match_label_expressions.iter().all(|requirement| {
        labels
            .get(&requirement.key)
            .is_some_and(|value| requirement.values.iter().any(|item| item == value))
    })
}
