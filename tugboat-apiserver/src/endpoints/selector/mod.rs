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
    pub(crate) fn try_parse(s: &str) -> Result<Vec<Selector>, Box<StatusResponse>> {
        let mut selectors = Vec::new();
        for fragment in s.split(',') {
            if let Some((key, value)) = fragment.split_once("!=") {
                if key.is_empty() {
                    return Err(Box::new(StatusResponse::bad_request(
                        "Invalid selector format: key must not be empty",
                        None,
                    )));
                }
                selectors.push(Selector::NotEqual(key.to_string(), value.to_string()));
            } else if let Some((key, value)) = fragment.split_once('=') {
                if key.is_empty() {
                    return Err(Box::new(StatusResponse::bad_request(
                        "Invalid selector format: key must not be empty",
                        None,
                    )));
                }
                selectors.push(Selector::Equal(key.to_string(), value.to_string()));
            } else {
                return Err(Box::new(StatusResponse::bad_request(
                    "Invalid selector format: must be key=value or key!=value, separated by commas",
                    None,
                )));
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
        let selector = "app.tugboat.cloud/name=nginx";
        let actual = Selector::try_parse(selector).expect("failed to parse");
        assert_eq!(
            actual,
            vec![Selector::Equal(
                "app.tugboat.cloud/name".to_string(),
                "nginx".to_string()
            )]
        );
    }

    #[test]
    fn can_parse_inequality() {
        let selector = "app.tugboat.cloud/name!=nginx";
        let actual = Selector::try_parse(selector).expect("failed to parse");
        assert_eq!(
            actual,
            vec![Selector::NotEqual(
                "app.tugboat.cloud/name".to_string(),
                "nginx".to_string()
            )]
        );
    }

    #[test]
    fn can_parse_multiple() {
        let selector = "app.tugboat.cloud/instance=abc,app.tugboat.cloud/name!=nginx,app.tugboat.cloud/component!=controller";
        let actual = Selector::try_parse(selector).expect("failed to parse");
        assert_eq!(
            actual,
            vec![
                Selector::Equal("app.tugboat.cloud/instance".to_string(), "abc".to_string()),
                Selector::NotEqual("app.tugboat.cloud/name".to_string(), "nginx".to_string()),
                Selector::NotEqual(
                    "app.tugboat.cloud/component".to_string(),
                    "controller".to_string()
                )
            ]
        );
    }
}
