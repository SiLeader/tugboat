use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorKind {
    Io,
    Serialization,
    Syscall,
    Network,
    Storage,
    VmOperation,
    Validation,
    Unknown,
}
