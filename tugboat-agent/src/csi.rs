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
use nix::sched::{CloneFlags, setns};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{OpenOptions, create_dir_all, read_dir, remove_dir, remove_file};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use tugboat_csi_operator::{CsiAccessMode, CsiAccessType, NodeCapability, TugboatCsiOperator};
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolumeClaimSpec, PersistentVolumeSpec,
};

pub(crate) const READ_ONLY_MANY: &str = "ReadOnlyMany";
pub(crate) const READ_WRITE_ONCE: &str = "ReadWriteOnce";
pub(crate) const READ_WRITE_MANY: &str = "ReadWriteMany";

const ACCESS_MODE_PRIORITY: [&str; 3] = [READ_WRITE_MANY, READ_WRITE_ONCE, READ_ONLY_MANY];

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
    #[error("Target path '{0}' is a directory")]
    TargetPathIsDirectory(String),
    #[error(
        "PersistentVolumeClaim access modes '{claim_access_modes}' are incompatible with PersistentVolume access modes '{volume_access_modes}'"
    )]
    IncompatibleAccessModes {
        claim_access_modes: String,
        volume_access_modes: String,
    },
    #[error("Unsupported driver capability: {0}")]
    UnsupportedNodeCapability(String),
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublishedVolume {
    pub driver: String,
    pub volume_id: String,
    pub target_path: String,
    pub mount_namespace_path: String,
}

#[derive(Clone)]
pub(crate) struct CsiWrapper {
    operator: TugboatCsiOperator,
    drivers: CsiDrivers,
    publish_dir: PathBuf,
}

impl CsiWrapper {
    pub(crate) fn new(
        operator: TugboatCsiOperator,
        drivers: CsiDrivers,
        publish_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            operator,
            drivers,
            publish_dir: publish_dir.into(),
        }
    }

    pub(crate) fn plan_published_volume(
        &self,
        ship_id: &str,
        claim_name: &str,
        source: &CsiPersistentVolumeSource,
    ) -> Result<PublishedVolume, CsiError> {
        if source.volume_handle.is_empty() {
            return Err(CsiError::MissingVolumeHandle);
        }
        Ok(PublishedVolume {
            driver: source.driver.clone(),
            volume_id: source.volume_handle.clone(),
            target_path: self.target_path(ship_id, claim_name),
            mount_namespace_path: mountns::path_for_ship(ship_id).display().to_string(),
        })
    }

    pub(crate) fn ensure_mount_namespace(&self, ship_id: &str) -> Result<String, CsiError> {
        Ok(mountns::ensure_for_ship(ship_id)?.display().to_string())
    }

    pub(crate) fn cleanup_mount_namespace(&self, ship_id: &str) -> Result<(), CsiError> {
        mountns::cleanup_for_ship(ship_id)?;
        Ok(())
    }

    pub(crate) fn load_published_volumes(
        &self,
        ship_id: &str,
    ) -> Result<Vec<PublishedVolume>, CsiError> {
        let ship_dir = self.publish_dir.join(ship_id);
        let entries = match read_dir(ship_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };

        let mut volumes: Vec<PublishedVolume> = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension() != Some(OsStr::new("json")) {
                continue;
            }

            let contents = std::fs::read(&path)?;
            volumes.push(serde_json::from_slice(&contents)?);
        }
        volumes.sort_by(|left, right| left.target_path.cmp(&right.target_path));
        Ok(volumes)
    }

    pub(crate) async fn publish(
        &self,
        ship_id: &str,
        claim_name: &str,
        volume: &PersistentVolumeSpec,
        claim: &PersistentVolumeClaimSpec,
        source: &CsiPersistentVolumeSource,
    ) -> Result<PublishedVolume, CsiError> {
        let Some(uds_path) = self.drivers.get(&source.driver) else {
            return Err(CsiError::DriverNotFound(source.driver.clone()));
        };
        let node_capabilities = self.operator.node_capabilities(uds_path).await?;
        ensure_supported_node_capabilities(&node_capabilities)?;
        let (access_mode, read_only) = effective_publish_settings(
            &claim.access_modes,
            &volume.access_modes,
            source.read_only,
        )?;
        let access_type = CsiAccessType::try_convert_from_string(
            volume.volume_mode.as_deref().unwrap_or("Block"),
        )?;
        self.ensure_mount_namespace(ship_id)?;
        let published = self.plan_published_volume(ship_id, claim_name, source)?;
        prepare_target_path(&published.target_path, &access_type)?;
        let operator = self.operator.clone();
        let uds_path = uds_path.to_string();
        let volume_id = published.volume_id.clone();
        let target_path = published.target_path.clone();
        let mount_namespace_path = published.mount_namespace_path.clone();
        run_in_mount_namespace(mount_namespace_path, move || async move {
            operator
                .publish(
                    &uds_path,
                    volume_id,
                    target_path,
                    read_only,
                    access_mode,
                    access_type,
                )
                .await
        })
        .await?;
        if let Err(err) = self.persist_published_volume(&published) {
            if let Err(cleanup_err) = self.unpublish(&published).await {
                tracing::error!(
                    "Failed to roll back published volume after state persistence error: {cleanup_err}"
                );
            }
            return Err(err);
        }
        Ok(published)
    }

    pub(crate) async fn unpublish(&self, volume: &PublishedVolume) -> Result<(), CsiError> {
        let Some(uds_path) = self.drivers.get(&volume.driver) else {
            return Err(CsiError::DriverNotFound(volume.driver.clone()));
        };

        let operator = self.operator.clone();
        let uds_path = uds_path.to_string();
        let volume_id = volume.volume_id.clone();
        let target_path = volume.target_path.clone();
        let mount_namespace_path = volume.mount_namespace_path.clone();
        match run_in_mount_namespace(mount_namespace_path, move || async move {
            operator.unpublish(&uds_path, volume_id, target_path).await
        })
        .await
        {
            Ok(()) | Err(CsiError::Driver(tugboat_csi_operator::Error::TargetPathNotFound)) => {}
            Err(err) => return Err(err),
        }

        cleanup_target_path(&volume.target_path)?;
        self.remove_published_volume_state(volume)?;
        Ok(())
    }

    fn target_path(&self, ship_id: &str, claim_name: &str) -> String {
        self.publish_dir
            .join(ship_id)
            .join(format!("{claim_name}.block"))
            .display()
            .to_string()
    }

    fn persist_published_volume(&self, volume: &PublishedVolume) -> Result<(), CsiError> {
        let path = self.state_path_for_volume(volume)?;
        let Some(parent) = path.parent() else {
            return Err(CsiError::TargetPathHasNoParent(path.display().to_string()));
        };
        create_dir_all(parent)?;
        std::fs::write(path, serde_json::to_vec(volume)?)?;
        Ok(())
    }

    fn remove_published_volume_state(&self, volume: &PublishedVolume) -> Result<(), CsiError> {
        let path = self.state_path_for_volume(volume)?;
        match remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let Some(parent) = path.parent() else {
            return Err(CsiError::TargetPathHasNoParent(path.display().to_string()));
        };
        match remove_dir(parent) {
            Ok(()) => {}
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }

    fn state_path_for_volume(&self, volume: &PublishedVolume) -> Result<PathBuf, CsiError> {
        let target_path = Path::new(&volume.target_path);
        let Some(parent) = target_path.parent() else {
            return Err(CsiError::TargetPathHasNoParent(volume.target_path.clone()));
        };
        let Some(stem) = target_path.file_stem() else {
            return Err(CsiError::TargetPathHasNoFileStem(
                volume.target_path.clone(),
            ));
        };
        Ok(parent.join(format!("{}.json", stem.to_string_lossy())))
    }
}

async fn run_in_mount_namespace<T, F, Fut>(
    mount_namespace_path: String,
    operation: F,
) -> Result<T, CsiError>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, tugboat_csi_operator::Error>> + 'static,
{
    tokio::task::spawn_blocking(move || -> Result<T, CsiError> {
        let current_namespace = std::fs::File::open("/proc/self/ns/mnt")?;
        let target_namespace = std::fs::File::open(&mount_namespace_path)?;
        setns(&target_namespace, CloneFlags::CLONE_NEWNS)?;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(operation()).map_err(CsiError::from);
        let restore_result = setns(&current_namespace, CloneFlags::CLONE_NEWNS);
        match (result, restore_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(err), Ok(())) => Err(err),
            (Ok(_), Err(err)) => Err(err.into()),
            (Err(err), Err(restore_err)) => {
                tracing::error!(
                    "Failed to restore mount namespace after CSI operation failed: {restore_err}"
                );
                Err(err)
            }
        }
    })
    .await
    .map_err(CsiError::from)?
}

fn prepare_target_path(target_path: &str, access_type: &CsiAccessType) -> Result<(), CsiError> {
    let path = Path::new(target_path);
    let Some(parent) = path.parent() else {
        return Err(CsiError::TargetPathHasNoParent(target_path.to_string()));
    };
    create_dir_all(parent)?;
    match access_type {
        CsiAccessType::Block => {
            if path.is_dir() {
                return Err(CsiError::TargetPathIsDirectory(target_path.to_string()));
            }
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)?;
        }
    }
    Ok(())
}

fn cleanup_target_path(target_path: &str) -> Result<(), CsiError> {
    let path = Path::new(target_path);
    match remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    let Some(parent) = path.parent() else {
        return Err(CsiError::TargetPathHasNoParent(target_path.to_string()));
    };
    match remove_dir(parent) {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

trait TryConvertFromString: Sized {
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
            _ => Err(CsiError::UnrecognizedAccessType(value.to_string())),
        }
    }
}

pub(crate) fn is_supported_access_mode(mode: &str) -> bool {
    matches!(mode, READ_ONLY_MANY | READ_WRITE_ONCE | READ_WRITE_MANY)
}

pub(crate) fn effective_publish_settings(
    claim_access_modes: &[String],
    volume_access_modes: &[String],
    source_read_only: bool,
) -> Result<(CsiAccessMode, bool), CsiError> {
    let access_mode = select_access_mode(claim_access_modes, volume_access_modes)?;
    Ok((
        access_mode,
        source_read_only || matches!(access_mode, CsiAccessMode::ReadOnlyMany),
    ))
}

fn select_access_mode(
    claim_access_modes: &[String],
    volume_access_modes: &[String],
) -> Result<CsiAccessMode, CsiError> {
    if claim_access_modes.is_empty() {
        return Err(CsiError::MissingAccessMode);
    }

    for candidate in ACCESS_MODE_PRIORITY {
        let requested = claim_access_modes
            .iter()
            .any(|mode| mode.as_str() == candidate);
        let supported = volume_access_modes
            .iter()
            .any(|mode| mode.as_str() == candidate);
        if requested && supported {
            return CsiAccessMode::try_convert_from_string(candidate);
        }
    }

    Err(CsiError::IncompatibleAccessModes {
        claim_access_modes: claim_access_modes.join(", "),
        volume_access_modes: volume_access_modes.join(", "),
    })
}

fn ensure_supported_node_capabilities(capabilities: &[NodeCapability]) -> Result<(), CsiError> {
    if capabilities.contains(&NodeCapability::StageUnstageVolume) {
        Err(CsiError::UnsupportedNodeCapability(
            "STAGE_UNSTAGE_VOLUME".to_string(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Default, Debug)]
pub struct CsiDrivers {
    drivers: HashMap<String, String>,
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

#[cfg(test)]
mod tests {
    use super::{
        CsiDrivers, CsiWrapper, PublishedVolume, TryConvertFromString, effective_publish_settings,
        select_access_mode,
    };
    use tugboat_csi_operator::{CsiAccessMode, CsiAccessType, TugboatCsiOperator};
    use tugboat_resources::manifests::core::v1::CsiPersistentVolumeSource;

    #[test]
    fn can_convert_access_modes() {
        assert!(matches!(
            CsiAccessMode::try_convert_from_string("ReadWriteOnce"),
            Ok(CsiAccessMode::ReadWriteOnce)
        ));
        assert!(matches!(
            CsiAccessType::try_convert_from_string("Block"),
            Ok(CsiAccessType::Block)
        ));
    }

    #[test]
    fn can_plan_publish_target_path() {
        let wrapper = CsiWrapper::new(
            TugboatCsiOperator::default(),
            CsiDrivers::default(),
            "/var/lib/tugboat-agent/csi",
        );
        let published = wrapper
            .plan_published_volume(
                "ship-uid",
                "data-volume",
                &CsiPersistentVolumeSource {
                    driver: "example.csi".to_string(),
                    volume_handle: "volume-001".to_string(),
                    ..Default::default()
                },
            )
            .expect("volume planning should succeed");

        assert_eq!(published.driver, "example.csi");
        assert_eq!(published.volume_id, "volume-001");
        assert_eq!(
            published.target_path,
            "/var/lib/tugboat-agent/csi/ship-uid/data-volume.block"
        );
        assert_eq!(
            published.mount_namespace_path,
            "/var/run/tugboat/mntns/ship-uid"
        );
    }

    #[test]
    fn chooses_effective_access_mode_without_list_order_dependency() {
        let access_mode = select_access_mode(
            &["ReadOnlyMany".to_string(), "ReadWriteOnce".to_string()],
            &["ReadWriteOnce".to_string(), "ReadOnlyMany".to_string()],
        )
        .expect("access mode selection should succeed");

        assert_eq!(access_mode, CsiAccessMode::ReadWriteOnce);
    }

    #[test]
    fn read_only_claims_force_read_only_publish() {
        let (_, read_only) = effective_publish_settings(
            &["ReadOnlyMany".to_string()],
            &["ReadOnlyMany".to_string()],
            false,
        )
        .expect("publish settings should succeed");

        assert!(read_only);
    }

    #[test]
    fn can_persist_and_load_published_volume_state() {
        let temp_dir = std::env::temp_dir().join(format!(
            "tugboat-agent-csi-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be monotonic")
                .as_nanos()
        ));
        let wrapper = CsiWrapper::new(
            TugboatCsiOperator::default(),
            CsiDrivers::default(),
            &temp_dir,
        );
        let volume = PublishedVolume {
            driver: "example.csi".to_string(),
            volume_id: "volume-001".to_string(),
            target_path: temp_dir
                .join("ship-uid")
                .join("data-volume.block")
                .display()
                .to_string(),
            mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        };

        wrapper
            .persist_published_volume(&volume)
            .expect("state persistence should succeed");

        let loaded = wrapper
            .load_published_volumes("ship-uid")
            .expect("state loading should succeed");

        assert_eq!(loaded, vec![volume.clone()]);

        wrapper
            .remove_published_volume_state(&volume)
            .expect("state cleanup should succeed");
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
