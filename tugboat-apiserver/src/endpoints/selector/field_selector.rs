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

use crate::endpoints::selector::Selector;
use serde::Serialize;

pub(crate) enum FieldSelector {
    Equal(Vec<String>, String),
    NotEqual(Vec<String>, String),
}

pub(crate) struct FieldSelectorFilter<I> {
    field_selector: Vec<FieldSelector>,
    iter: I,
}

pub(crate) trait FilterByFieldSelector: Sized {
    fn filter_by_field_selector(self, field_selector: Vec<Selector>) -> FieldSelectorFilter<Self>;
}

impl<I> FilterByFieldSelector for I
where
    I: Iterator,
{
    fn filter_by_field_selector(self, field_selector: Vec<Selector>) -> FieldSelectorFilter<Self> {
        FieldSelectorFilter {
            field_selector: field_selector.into_iter().map(Into::into).collect(),
            iter: self,
        }
    }
}

impl From<Selector> for FieldSelector {
    fn from(value: Selector) -> Self {
        match value {
            Selector::Equal(key, value) => {
                FieldSelector::Equal(key.split('.').map(ToString::to_string).collect(), value)
            }
            Selector::NotEqual(key, value) => {
                FieldSelector::NotEqual(key.split('.').map(ToString::to_string).collect(), value)
            }
        }
    }
}

impl FieldSelector {
    pub(crate) fn is_match(&self, value: &serde_json::Value) -> bool {
        let (key, expected, is_equal) = match self {
            FieldSelector::Equal(key, v) => (key, v, true),
            FieldSelector::NotEqual(key, v) => (key, v, false),
        };

        let Some(value) = follow(value, 0, key.as_slice()) else {
            return if is_equal {
                expected.is_empty()
            } else {
                !expected.is_empty()
            };
        };

        if is_equal {
            value == expected || (expected.is_empty() && value.is_null())
        } else {
            value != expected
        }
    }
}

impl<I> Iterator for FieldSelectorFilter<I>
where
    I: Iterator,
    I::Item: Serialize,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let whole_value = self.iter.next()?;
            let Ok(value) = serde_json::to_value(&whole_value) else {
                continue;
            };
            if self.field_selector.iter().all(|sel| sel.is_match(&value)) {
                return Some(whole_value);
            }
        }
    }
}

fn follow<'a>(
    value: &'a serde_json::Value,
    index: usize,
    route: &[String],
) -> Option<&'a serde_json::Value> {
    if route.len() == index {
        return Some(value);
    }
    let obj = value.as_object()?;
    let data = obj.get(route.get(index)?)?;
    follow(data, index + 1, route)
}

#[cfg(test)]
mod tests {
    use super::FieldSelector;
    use crate::endpoints::selector::Selector;
    use crate::endpoints::selector::field_selector::FilterByFieldSelector;
    use tugboat_resources::manifests::core::v1::{Ship, ShipSpec};

    #[test]
    fn can_be_filtered_by_field_selector() {
        let contents = vec![
            Ship {
                spec: Some(ShipSpec {
                    node_name: None,
                    ..ShipSpec::default()
                }),
                ..Ship::default()
            },
            Ship {
                spec: Some(ShipSpec {
                    node_name: None,
                    ..ShipSpec::default()
                }),
                ..Ship::default()
            },
            Ship {
                spec: Some(ShipSpec {
                    node_name: Some("node1".to_string()),
                    ..ShipSpec::default()
                }),
                ..Ship::default()
            },
            Ship {
                spec: Some(ShipSpec {
                    node_name: Some("node1".to_string()),
                    ..ShipSpec::default()
                }),
                ..Ship::default()
            },
        ];

        let actual = contents
            .iter()
            .filter_by_field_selector(vec![Selector::Equal(
                "spec.nodeName".to_string(),
                "node1".to_string(),
            )])
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), 2);

        let actual = contents
            .iter()
            .filter_by_field_selector(vec![Selector::Equal(
                "spec.nodeName".to_string(),
                "node".to_string(),
            )])
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), 0);

        let actual = contents
            .into_iter()
            .filter_by_field_selector(vec![Selector::Equal(
                "spec.nodeName".to_string(),
                "".to_string(),
            )])
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), 2);
    }

    #[test]
    fn test_missing_field_matching_logic() {
        let ship = Ship {
            spec: Some(ShipSpec {
                node_name: None,
                ..ShipSpec::default()
            }),
            ..Ship::default()
        };
        let val = serde_json::to_value(&ship).unwrap();

        // Case 1: field == "" (should match)
        let s1 = FieldSelector::Equal(vec!["spec".to_string(), "nodeName".to_string()], "".to_string());
        assert!(s1.is_match(&val), "Missing field should match == ''");

        // Case 2: field == "foo" (should not match)
        let s2 = FieldSelector::Equal(vec!["spec".to_string(), "nodeName".to_string()], "foo".to_string());
        assert!(!s2.is_match(&val), "Missing field should NOT match == 'foo'");

        // Case 3: field != "" (should not match, because "" == "")
        let s3 = FieldSelector::NotEqual(vec!["spec".to_string(), "nodeName".to_string()], "".to_string());
        assert!(!s3.is_match(&val), "Missing field should NOT match != ''");

        // Case 4: field != "foo" (should match, because "" != "foo")
        let s4 = FieldSelector::NotEqual(vec!["spec".to_string(), "nodeName".to_string()], "foo".to_string());
        assert!(s4.is_match(&val), "Missing field should match != 'foo'");

        // Case 5: intermediate object missing
        let ship_no_spec = Ship {
            spec: None,
            ..Ship::default()
        };
        let val_no_spec = serde_json::to_value(&ship_no_spec).unwrap();
        let s5 = FieldSelector::Equal(vec!["spec".to_string(), "nodeName".to_string()], "".to_string());
        assert!(s5.is_match(&val_no_spec), "Missing intermediate field should match == ''");
    }
}
