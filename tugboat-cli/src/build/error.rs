#[derive(Debug)]
pub(super) enum BuildError {
    InvalidFormat(String),
    InvalidArch(String),
}
