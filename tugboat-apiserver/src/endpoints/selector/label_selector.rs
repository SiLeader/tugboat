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
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::meta::v1::ObjectMeta;

pub(crate) struct LabelSelectorFilter<I> {
    label_selector: Vec<Selector>,
    iter: I,
}

pub(crate) trait FilterByLabelSelector: Sized {
    fn filter_by_label_selector(self, label_selector: Vec<Selector>) -> LabelSelectorFilter<Self>;
}

impl<I> FilterByLabelSelector for I
where
    I: Iterator,
{
    fn filter_by_label_selector(self, label_selector: Vec<Selector>) -> LabelSelectorFilter<Self> {
        LabelSelectorFilter {
            label_selector,
            iter: self,
        }
    }
}

impl Selector {
    pub(crate) fn is_label_match(&self, meta: &ObjectMeta) -> bool {
        match self {
            Selector::Equal(key, value) => {
                let Some(actual_value) = meta.labels.get(key) else {
                    return false;
                };
                actual_value == value
            }
            Selector::NotEqual(key, value) => {
                let Some(actual_value) = meta.labels.get(key) else {
                    return false;
                };
                actual_value != value
            }
        }
    }
}

impl<I> Iterator for LabelSelectorFilter<I>
where
    I: Iterator,
    I::Item: ObjectMetaResource,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.iter.next()?;
            let Some(meta) = item.object_meta() else {
                continue;
            };
            if self
                .label_selector
                .iter()
                .all(|sel| sel.is_label_match(meta))
            {
                return Some(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::endpoints::selector::Selector;
    use crate::endpoints::selector::label_selector::FilterByLabelSelector;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::Ship;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn can_be_filtered_by_labels() {
        let contents = vec![
            Ship {
                object_meta: Some(ObjectMeta {
                    labels: HashMap::from_iter([
                        ("track".to_string(), "stable".to_string()),
                        ("environment".to_string(), "production".to_string()),
                    ]),
                    ..ObjectMeta::default()
                }),
                ..Ship::default()
            },
            Ship {
                object_meta: Some(ObjectMeta {
                    labels: HashMap::from_iter([
                        ("track".to_string(), "stable".to_string()),
                        ("environment".to_string(), "development".to_string()),
                    ]),
                    ..ObjectMeta::default()
                }),
                ..Ship::default()
            },
            Ship {
                object_meta: Some(ObjectMeta {
                    labels: HashMap::from_iter([
                        ("track".to_string(), "testing".to_string()),
                        ("environment".to_string(), "production".to_string()),
                    ]),
                    ..ObjectMeta::default()
                }),
                ..Ship::default()
            },
            Ship {
                object_meta: Some(ObjectMeta {
                    labels: HashMap::from_iter([
                        ("track".to_string(), "testing".to_string()),
                        ("environment".to_string(), "development".to_string()),
                    ]),
                    ..ObjectMeta::default()
                }),
                ..Ship::default()
            },
        ];

        let actual = contents
            .into_iter()
            .filter_by_label_selector(vec![Selector::Equal(
                "environment".to_string(),
                "production".to_string(),
            )])
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), 2);
    }
}
