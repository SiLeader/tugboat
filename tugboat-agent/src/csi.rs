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
use std::collections::HashMap;
use std::fs::{OpenOptions, create_dir_all, remove_dir, remove_file};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use tugboat_csi_operator::{CsiAccessMode, CsiAccessType, TugboatCsiOperator};
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolumeClaimSpec, PersistentVolumeSpec,
};

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
    #[error("Target path '{0}' is a directory")]
    TargetPathIsDirectory(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("System call error: {0}")]
    Syscall(#[from] nix::errno::Errno),
    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("Mount namespace error: {0}")]
    MountNamespace(#[from] mountns::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
        let access_mode = CsiAccessMode::try_convert_from_string(
            claim
                .access_modes
                .first()
                .ok_or(CsiError::MissingAccessMode)?,
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
        let read_only = source.read_only;
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
            Err(err) => return Err(err.into()),
        }

        cleanup_target_path(&volume.target_path)?;
        Ok(())
    }

    fn target_path(&self, ship_id: &str, claim_name: &str) -> String {
        self.publish_dir
            .join(ship_id)
            .join(format!("{claim_name}.block"))
            .display()
            .to_string()
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
            "ReadOnlyMany" => Ok(CsiAccessMode::ReadOnlyMany),
            "ReadWriteOnce" => Ok(CsiAccessMode::ReadWriteOnce),
            "ReadWriteMany" => Ok(CsiAccessMode::ReadWriteMany),
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
    use super::{CsiDrivers, CsiWrapper, TryConvertFromString};
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
}
