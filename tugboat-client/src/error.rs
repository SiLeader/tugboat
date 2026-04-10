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

use serde::Deserialize;
use std::fmt::{Display, Formatter};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("credentials must not be sent over insecure URL: {0}")]
    InsecureUrl(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(#[from] ApiStatus),
    #[error("Line parse error: {0}")]
    LineParse(#[from] tokio_util::codec::LinesCodecError),
    #[error("Deserialize error: {0}")]
    Deserialize(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),
    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),
}

#[derive(Debug, Deserialize, thiserror::Error)]
pub struct ApiStatus {
    pub status: String,
    pub message: String,
    pub reason: String,
    pub code: u16,
    pub details: Option<serde_json::Value>,
}

impl Display for ApiStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "status = {}, code = {}, reason = {}, message = {}",
            self.status, self.code, self.reason, self.message
        )
    }
}
