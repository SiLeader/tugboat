use crate::endpoints::selector::Selector;
use tugboat_resources::ObjectMetaResource;

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
            for selector in &self.label_selector {
                match selector {
                    Selector::Equal(key, value) => {
                        let Some(actual_value) = meta.labels.get(key) else {
                            continue;
                        };
                        if actual_value == value {
                            return Some(item);
                        }
                    }
                    Selector::NotEqual(key, value) => {
                        let Some(actual_value) = meta.labels.get(key) else {
                            continue;
                        };
                        if actual_value != value {
                            return Some(item);
                        }
                    }
                }
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
