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

use crate::manifests::core::v1::VolumeNodeAffinity;
use crate::validators::Validator;

const VALID_NODE_SELECTOR_OPERATORS: &[&str] =
    &["In", "NotIn", "Exists", "DoesNotExist", "Gt", "Lt"];

pub trait HasNodeAffinity {
    fn node_affinity_value(&self) -> Option<&VolumeNodeAffinity>;
}

pub struct NodeAffinityValidator;

impl<T> Validator<T> for NodeAffinityValidator
where
    T: HasNodeAffinity,
{
    fn validate(&self, value: &T) -> bool {
        let Some(affinity) = value.node_affinity_value() else {
            return true;
        };
        let Some(required) = affinity.required.as_ref() else {
            return true;
        };

        required.node_selector_terms.iter().all(|term| {
            term.match_expressions
                .iter()
                .chain(term.match_fields.iter())
                .all(|requirement| {
                    !requirement.key.is_empty()
                        && VALID_NODE_SELECTOR_OPERATORS.contains(&requirement.operator.as_str())
                })
        })
    }
}
