use serde::Deserialize;
use std::fmt::{Display, Formatter};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
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
}

#[derive(Debug, Deserialize, thiserror::Error)]
pub struct ApiStatus {
    status: String,
    message: String,
    reason: String,
    code: u16,
    details: Option<serde_json::Value>,
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
