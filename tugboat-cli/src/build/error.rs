use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum BuildError {
    #[error("Invalid Imagefile format: {0}")]
    InvalidFormat(String),
    #[error("Invalid Imagefile arch: {0}")]
    InvalidArch(String),
}
