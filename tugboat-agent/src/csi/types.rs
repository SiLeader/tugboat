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

use crate::mountns;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use tugboat_csi_operator::{CsiAccessMode, CsiAccessType};

pub(crate) const READ_ONLY_MANY: &str = "ReadOnlyMany";
pub(crate) const READ_WRITE_ONCE: &str = "ReadWriteOnce";
pub(crate) const READ_WRITE_MANY: &str = "ReadWriteMany";

pub(crate) const ACCESS_MODE_PRIORITY: [&str; 3] =
    [READ_WRITE_MANY, READ_WRITE_ONCE, READ_ONLY_MANY];

#[derive(Debug, thiserror::Error)]
pub(crate) enum CsiError {
    #[error("Driver error: {0}")]
    Driver(#[from] tugboat_csi_operator::Error),
    #[error("Driver '{0}' not found")]
    DriverNotFound(String),
    #[error("Unrecognized access mode: {0}")]
    UnrecognizedAccessMode(String),
    #[error("Unrecognized access type: {0}")]
    UnrecognizedAccessType(String),
    #[error("Access mode is missing in claim spec")]
    MissingAccessMode,
    #[error("CSI volume handle is missing")]
    MissingVolumeHandle,
    #[error("Target path '{0}' has no parent directory")]
    TargetPathHasNoParent(String),
    #[error("Target path '{0}' has no file stem")]
    TargetPathHasNoFileStem(String),

    #[error(
        "PersistentVolumeClaim access modes '{claim_access_modes}' are incompatible with PersistentVolume access modes '{volume_access_modes}'"
    )]
    IncompatibleAccessModes {
        claim_access_modes: String,
        volume_access_modes: String,
    },
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("System call error: {0}")]
    Syscall(#[from] nix::errno::Errno),
    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("Mount namespace error: {0}")]
    MountNamespace(#[from] mountns::Error),
    #[error(
        "CSI operation failed and rollback also failed ({context}); original: {original}; rollback: {rollback}"
    )]
    RollbackFailed {
        context: String,
        original: String,
        rollback: String,
    },
    #[error(
        "Failed to persist published volume state for '{volume_id}' (mounted on node but state file write failed): {reason}"
    )]
    PublishPartialState {
        volume_id: String,
        reason: String,
        published: Box<PublishedVolume>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PublishedAccessType {
    #[default]
    Block,
    Filesystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublishedVolume {
    pub claim_name: String,
    pub driver: String,
    pub volume_id: String,
    pub target_path: String,
    #[serde(default)]
    pub access_type: PublishedAccessType,
    pub mount_namespace_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_target_path: Option<String>,
    #[serde(default)]
    pub controller_published: bool,
    /// Actual PersistentVolumeClaim resource name. `claim_name` stores the
    /// Ship-spec volume alias used for target paths, which may differ from the
    /// real PVC name. Old persisted state omits this field; callers should use
    /// [`Self::effective_pvc_name`] to fall back to `claim_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvc_name: Option<String>,
}

impl PublishedVolume {
    /// Returns the actual PVC resource name, falling back to `claim_name` for
    /// volumes persisted before the `pvc_name` field was introduced.
    pub fn effective_pvc_name(&self) -> &str {
        self.pvc_name.as_deref().unwrap_or(&self.claim_name)
    }

    /// Extracts the ship ID from the mount namespace path.
    /// Mount namespace path is typically `/var/run/tugboat/mntns/{ship-id}`.
    pub fn extract_ship_id(&self) -> Option<&str> {
        let path = std::path::Path::new(&self.mount_namespace_path);
        path.file_name()?.to_str()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedCsiSecrets {
    pub controller_publish: HashMap<String, String>,
    pub node_expand: HashMap<String, String>,
    pub node_publish: HashMap<String, String>,
    pub node_stage: HashMap<String, String>,
    pub mount_flags: Vec<String>,
}

#[derive(Clone, Default, Debug)]
pub struct CsiDrivers {
    pub(crate) drivers: HashMap<String, String>,
}

impl CsiDrivers {
    pub fn get(&self, driver: &str) -> Option<&str> {
        self.drivers.get(driver).map(|s| s.as_str())
    }
}

impl From<HashMap<String, String>> for CsiDrivers {
    fn from(drivers: HashMap<String, String>) -> Self {
        Self { drivers }
    }
}

pub(crate) trait TryConvertFromString: Sized {
    fn try_convert_from_string(value: &str) -> Result<Self, CsiError>;
}

impl TryConvertFromString for CsiAccessMode {
    fn try_convert_from_string(value: &str) -> Result<Self, CsiError> {
        match value {
            READ_ONLY_MANY => Ok(CsiAccessMode::ReadOnlyMany),
            READ_WRITE_ONCE => Ok(CsiAccessMode::ReadWriteOnce),
            READ_WRITE_MANY => Ok(CsiAccessMode::ReadWriteMany),
            _ => Err(CsiError::UnrecognizedAccessMode(value.to_string())),
        }
    }
}

impl TryConvertFromString for CsiAccessType {
    fn try_convert_from_string(value: &str) -> Result<Self, CsiError> {
        match value {
            "Block" => Ok(CsiAccessType::Block),
            "Filesystem" => Ok(CsiAccessType::Filesystem),
            _ => Err(CsiError::UnrecognizedAccessType(value.to_string())),
        }
    }
}

impl From<CsiAccessType> for PublishedAccessType {
    fn from(value: CsiAccessType) -> Self {
        match value {
            CsiAccessType::Block => Self::Block,
            CsiAccessType::Filesystem => Self::Filesystem,
        }
    }
}
