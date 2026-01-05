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
    pub fn as_byte_length(&self) -> Option<u64> {
        let captures = SIZE_REGEX.captures(self.0.as_str())?;
        let size_number = u64::from_str(captures.get(1)?.as_str()).ok()?;
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
