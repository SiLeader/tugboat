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

use crate::ObjectMetaResource;
use crate::validators::Validator;
use regex::Regex;
use std::sync::LazyLock;

pub struct NameValidator;

static NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^[a-z0-9]([-a-z0-9]*[a-z0-9])?$").unwrap());
static GENERATE_NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^[a-z0-9][-a-z0-9]*?$").unwrap());

impl<T> Validator<T> for NameValidator
where
    T: ObjectMetaResource,
{
    fn validate(&self, value: &T) -> bool {
        let Some(meta) = value.object_meta() else {
            return false;
        };
        if let Some(name) = &meta.name {
            Self::is_valid_name(name)
        } else if let Some(generate_name) = &meta.generate_name {
            Self::is_valid_generate_name(generate_name)
        } else {
            false
        }
    }
}

impl NameValidator {
    pub fn is_valid_name(name: &str) -> bool {
        !name.is_empty() && name.len() <= 253 && NAME_REGEX.is_match(name)
    }

    pub fn is_valid_generate_name(name: &str) -> bool {
        !name.is_empty() && name.len() <= (253 - 5) && GENERATE_NAME_REGEX.is_match(name)
    }
}

#[cfg(test)]
mod tests {
    use super::NameValidator;

    #[test]
    fn validates_resource_names() {
        assert!(NameValidator::is_valid_name("valid-name-1"));
        assert!(!NameValidator::is_valid_name(""));
        assert!(!NameValidator::is_valid_name("a/b"));
        assert!(!NameValidator::is_valid_name("BadName"));
    }

    #[test]
    fn validates_generate_names() {
        assert!(NameValidator::is_valid_generate_name("valid-prefix-"));
        assert!(!NameValidator::is_valid_generate_name(""));
        assert!(!NameValidator::is_valid_generate_name("bad/prefix-"));
    }
}
