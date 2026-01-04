use crate::data::StatusResponse;
use serde::Serialize;

mod field_selector;
mod label_selector;

pub(super) use field_selector::*;
pub(super) use label_selector::*;
use tugboat_resources::ObjectMetaResource;

#[derive(Debug, Eq, PartialEq, Clone)]
pub(crate) enum Selector {
    Equal(String, String),
    NotEqual(String, String),
}

impl Selector {
    pub(crate) fn try_parse(s: &str) -> Result<Vec<Selector>, StatusResponse> {
        let mut selectors = Vec::new();
        for fragment in s.split(',') {
            if let Some((key, value)) = fragment.split_once("!=") {
                selectors.push(Selector::NotEqual(key.to_string(), value.to_string()));
            } else if let Some((key, value)) = fragment.split_once("=") {
                selectors.push(Selector::Equal(key.to_string(), value.to_string()));
            } else {
                return Err(StatusResponse::bad_request(
                    "Invalid selector format: must be key=value or key!=value, separated by commas",
                    None,
                ));
            }
        }
        Ok(selectors)
    }
}

pub(crate) trait FilterBySelector: Iterator {
    fn filter_by_selector(
        self,
        field_selector: Option<Vec<Selector>>,
        label_selector: Option<Vec<Selector>>,
    ) -> Vec<Self::Item>;
}

impl<I> FilterBySelector for I
where
    I: Iterator,
    I::Item: Serialize + ObjectMetaResource,
{
    fn filter_by_selector(
        self,
        field_selector: Option<Vec<Selector>>,
        label_selector: Option<Vec<Selector>>,
    ) -> Vec<Self::Item> {
        match field_selector {
            None => match label_selector {
                None => self.collect(),
                Some(ls) => self.filter_by_label_selector(ls).collect(),
            },
            Some(fs) => match label_selector {
                None => self.filter_by_field_selector(fs).collect(),
                Some(ls) => self
                    .filter_by_label_selector(ls)
                    .filter_by_field_selector(fs)
                    .collect(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::endpoints::selector::Selector;

    #[test]
    fn can_parse_equality() {
        let selector = "app.kubernetes.io/name=nginx";
        let actual = Selector::try_parse(selector).expect("failed to parse");
        assert_eq!(
            actual,
            vec![Selector::Equal(
                "app.kubernetes.io/name".to_string(),
                "nginx".to_string()
            )]
        );
    }

    #[test]
    fn can_parse_inequality() {
        let selector = "app.kubernetes.io/name!=nginx";
        let actual = Selector::try_parse(selector).expect("failed to parse");
        assert_eq!(
            actual,
            vec![Selector::NotEqual(
                "app.kubernetes.io/name".to_string(),
                "nginx".to_string()
            )]
        );
    }

    #[test]
    fn can_parse_multiple() {
        let selector = "app.kubernetes.io/instance=abc,app.kubernetes.io/name!=nginx,app.kubernetes.io/component!=controller";
        let actual = Selector::try_parse(selector).expect("failed to parse");
        assert_eq!(
            actual,
            vec![
                Selector::Equal("app.kubernetes.io/instance".to_string(), "abc".to_string()),
                Selector::NotEqual("app.kubernetes.io/name".to_string(), "nginx".to_string()),
                Selector::NotEqual(
                    "app.kubernetes.io/component".to_string(),
                    "controller".to_string()
                )
            ]
        );
    }
}
