use regex::Regex;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizedString(pub String);

impl Default for SizedString {
    fn default() -> Self {
        Self("1Gi".to_string())
    }
}

static SIZE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)(Ki|Mi|Gi|Ti|Pi|K|M|G|T|P)$").unwrap());

impl SizedString {
    pub fn as_byte_length(&self) -> Option<usize> {
        let captures = SIZE_REGEX.captures(self.0.as_str())?;
        let size_number = usize::from_str(captures.get(1)?.as_str()).ok()?;
        let size_suffix = captures.get(2)?.as_str();

        let size = size_number
            * match size_suffix {
                "Ki" => 1024,
                "Mi" => 1024 * 1024,
                "Gi" => 1024 * 1024 * 1024,
                "Ti" => 1024 * 1024 * 1024 * 1024,
                "Pi" => 1024 * 1024 * 1024 * 1024 * 1024,
                "K" => 1000,
                "M" => 1000 * 1000,
                "G" => 1000 * 1000 * 1000,
                "T" => 1000 * 1000 * 1000 * 1000,
                "P" => 1000 * 1000 * 1000 * 1000 * 1000,
                _ => 1,
            };

        Some(size)
    }
}
