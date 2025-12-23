use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum QemuVmError {
    #[error("Failed to copy image: {0}")]
    Copy(#[from] std::io::Error),
}
