use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum RuntimeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Image error: {0}")]
    Image(#[from] tugboat_vm_image::Error),
    #[error("JSON Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Run VM error")]
    RunVm,
    #[error("Invalid memory size: {0}")]
    MemorySize(String),
}
