use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason")]
pub enum VmRunningStatus {
    Paused(VmPausedReason),
    Running,
    Shutdown,
    Suspended,
    Panicked,
    Error(VmErrorReason),
    Prelaunch,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum VmPausedReason {
    Stopped,
    InMigrating,
    FinishMigrating,
    PostMigration,
    Watchdog,
    Saving,
    Restoring,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum VmErrorReason {
    InternalError,
    IoError,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VmStatus {
    #[serde(flatten)]
    pub status: VmRunningStatus,
}
