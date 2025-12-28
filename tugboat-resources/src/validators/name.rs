use regex::Regex;
use std::sync::{LazyLock, OnceLock};

pub struct NameValidator;

static NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^[a-z0-9]([-a-z0-9]*[a-z0-9])?$").unwrap());

impl NameValidator {
    pub fn validate_name(&self, name: &str) -> bool {
        0 < name.len() && name.len() <= 253 && NAME_REGEX.is_match(name)
    }
}
