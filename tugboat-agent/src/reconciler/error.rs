use crate::runtime::error::RuntimeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ReconcileError {
    #[error("API error: {0}")]
    Api(#[from] tugboat_client::Error),
    #[error("Field '{1}' in '{0}' is missing")]
    FieldMissing(String, String),
    #[error("ShipClass '{0}' not found")]
    ShipClassNotFound(String),
    #[error("Runtime error: {0}")]
    Runtime(#[from] RuntimeError),
}
