use crate::endpoints::selector::Selector;
use serde::Serialize;

enum FieldSelector {
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
            for selector in &self.field_selector {
                let (key, expected, is_equal) = match selector {
                    FieldSelector::Equal(key, v) => (key, v, true),
                    FieldSelector::NotEqual(key, v) => (key, v, false),
                };

                let Some(value) = follow(&value, 0, key.as_slice()) else {
                    continue;
                };

                if is_equal {
                    if value == expected || (expected.is_empty() && value.is_null()) {
                        return Some(whole_value);
                    }
                } else {
                    if value != expected {
                        return Some(whole_value);
                    }
                }
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
    let Some(obj) = value.as_object() else {
        return None;
    };
    let data = obj.get(route.get(index)?)?;
    follow(data, index + 1, route)
}

#[cfg(test)]
mod tests {
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
}
