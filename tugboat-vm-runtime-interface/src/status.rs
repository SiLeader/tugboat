use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum VmStatus {
    Paused,
    Running,
    Shutdown,
    Suspended,
    Panicked,
    Error,
    Prelaunch,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VmStatusResponse {
    pub status: VmStatus,
    pub message: String,
}
