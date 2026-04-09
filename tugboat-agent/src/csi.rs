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

pub(crate) mod types;

use crate::mountns;
use nix::sched::{CloneFlags, setns};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{OpenOptions, create_dir_all, read_dir, remove_dir, remove_file};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use tugboat_csi_operator::{
    ControllerCapability, CsiAccessMode, CsiAccessType, NodeCapability, NodeVolumeStats,
    TugboatCsiOperator,
};
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolumeClaimSpec, PersistentVolumeSpec,
};

pub(crate) use types::{
    ACCESS_MODE_PRIORITY, CsiDrivers, CsiError, PublishedAccessType, PublishedVolume,
    READ_ONLY_MANY, READ_WRITE_MANY, READ_WRITE_ONCE, ResolvedCsiSecrets, TryConvertFromString,
};

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
        volume_alias: &str,
        pvc_name: &str,
        source: &CsiPersistentVolumeSource,
        access_type: PublishedAccessType,
        requires_staging: bool,
    ) -> Result<PublishedVolume, CsiError> {
        if source.volume_handle.is_empty() {
            return Err(CsiError::MissingVolumeHandle);
        }
        Ok(PublishedVolume {
            claim_name: volume_alias.to_string(),
            driver: source.driver.clone(),
            volume_id: source.volume_handle.clone(),
            target_path: self.target_path(ship_id, volume_alias, access_type),
            access_type,
            mount_namespace_path: mountns::path_for_ship(ship_id).display().to_string(),
            staging_target_path: requires_staging
                .then(|| self.staging_target_path(ship_id, volume_alias)),
            controller_published: false,
            pvc_name: Some(pvc_name.to_string()),
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn publish(
        &self,
        node_name: &str,
        ship_id: &str,
        volume_alias: &str,
        pvc_name: &str,
        volume: &PersistentVolumeSpec,
        claim: &PersistentVolumeClaimSpec,
        source: &CsiPersistentVolumeSource,
        secrets: &ResolvedCsiSecrets,
    ) -> Result<PublishedVolume, CsiError> {
        let Some(uds_path) = self.drivers.get(&source.driver) else {
            return Err(CsiError::DriverNotFound(source.driver.clone()));
        };
        let node_capabilities = self.operator.node_capabilities(uds_path).await?;
        let (access_mode, read_only) = effective_publish_settings(
            &claim.access_modes,
            &volume.access_modes,
            source.read_only,
        )?;
        let access_type = access_type_from_volume_mode(volume.volume_mode.as_deref())?;
        let fs_type = filesystem_type(source, access_type);
        let mount_flags = if matches!(access_type, CsiAccessType::Filesystem) {
            secrets.mount_flags.clone()
        } else {
            Vec::new()
        };
        let volume_context = source.volume_attributes.clone();
        let requires_staging = node_capabilities.contains(&NodeCapability::StageUnstageVolume);
        let controller_capabilities = self.operator.controller_capabilities(uds_path).await?;
        self.ensure_mount_namespace(ship_id)?;
        let mut published = self.plan_published_volume(
            ship_id,
            volume_alias,
            pvc_name,
            source,
            PublishedAccessType::from(access_type),
            requires_staging,
        )?;
        let publish_context =
            if controller_capabilities.contains(&ControllerCapability::PublishUnpublishVolume) {
                published.controller_published = true;
                self.operator
                    .controller_publish(
                        uds_path,
                        published.volume_id.clone(),
                        node_name.to_string(),
                        read_only,
                        access_mode,
                        access_type,
                        fs_type.clone(),
                        secrets.controller_publish.clone(),
                        volume_context.clone(),
                    )
                    .await?
            } else {
                HashMap::new()
            };
        let mut staged = false;
        if let Some(staging_target_path) = &published.staging_target_path {
            prepare_directory_path(staging_target_path)?;
            let operator = self.operator.clone();
            let uds_path = uds_path.to_string();
            let uds_path_for_controller = uds_path.clone();
            let volume_id = published.volume_id.clone();
            let staging_target_path_for_rpc = staging_target_path.clone();
            let mount_namespace_path = published.mount_namespace_path.clone();
            let node_stage_secrets = secrets.node_stage.clone();
            let fs_type = fs_type.clone();
            let mount_flags = mount_flags.clone();
            let volume_context = volume_context.clone();
            let publish_context = publish_context.clone();
            match run_in_mount_namespace(mount_namespace_path, move || async move {
                operator
                    .stage(
                        &uds_path,
                        volume_id,
                        staging_target_path_for_rpc,
                        access_mode,
                        access_type,
                        fs_type,
                        mount_flags,
                        node_stage_secrets,
                        volume_context,
                        publish_context,
                    )
                    .await
            })
            .await
            {
                Ok(_) => {}
                Err(err) => {
                    if published.controller_published {
                        let _ = self
                            .operator
                            .controller_unpublish(
                                &uds_path_for_controller,
                                published.volume_id.clone(),
                                node_name.to_string(),
                                secrets.controller_publish.clone(),
                            )
                            .await;
                    }
                    let _ = cleanup_directory_path(staging_target_path);
                    return Err(err);
                }
            }
            staged = true;
        }
        if let Err(err) = prepare_target_path(&published.target_path, published.access_type) {
            if staged {
                self.rollback_published_volume(
                    node_name,
                    &published,
                    &secrets.controller_publish,
                    "staged volume after target-path preparation error",
                )
                .await;
            }
            return Err(err);
        }
        let operator = self.operator.clone();
        let uds_path = uds_path.to_string();
        let volume_id = published.volume_id.clone();
        let target_path = published.target_path.clone();
        let mount_namespace_path = published.mount_namespace_path.clone();
        let staging_target_path = published.staging_target_path.clone();
        let node_publish_secrets = secrets.node_publish.clone();
        let fs_type = fs_type.clone();
        let mount_flags = mount_flags.clone();
        let volume_context = volume_context.clone();
        let publish_context = publish_context.clone();
        if let Err(err) = run_in_mount_namespace(mount_namespace_path, move || async move {
            operator
                .publish(
                    &uds_path,
                    volume_id,
                    target_path,
                    read_only,
                    access_mode,
                    access_type,
                    fs_type,
                    mount_flags,
                    staging_target_path,
                    node_publish_secrets,
                    volume_context,
                    publish_context,
                )
                .await
        })
        .await
        {
            self.rollback_published_volume(
                node_name,
                &published,
                &secrets.controller_publish,
                "staged/published volume after publish error",
            )
            .await;
            return Err(err);
        }
        if let Err(err) = self.persist_published_volume(&published) {
            self.rollback_published_volume(
                node_name,
                &published,
                &secrets.controller_publish,
                "published volume after state persistence error",
            )
            .await;
            return Err(err);
        }
        Ok(published)
    }

    pub(crate) async fn unpublish(
        &self,
        volume: &PublishedVolume,
        node_name: &str,
        controller_publish_secrets: &HashMap<String, String>,
    ) -> Result<(), CsiError> {
        let Some(uds_path) = self.drivers.get(&volume.driver) else {
            return Err(CsiError::DriverNotFound(volume.driver.clone()));
        };

        let operator = self.operator.clone();
        let uds_path = uds_path.to_string();
        let volume_id = volume.volume_id.clone();
        let target_path = volume.target_path.clone();
        let mount_namespace_path = volume.mount_namespace_path.clone();
        let uds_path_for_unpublish = uds_path.clone();
        match run_in_mount_namespace(mount_namespace_path, move || async move {
            operator
                .unpublish(&uds_path_for_unpublish, volume_id, target_path)
                .await
        })
        .await
        {
            Ok(()) | Err(CsiError::Driver(tugboat_csi_operator::Error::TargetPathNotFound)) => {}
            Err(err) => return Err(err),
        }

        cleanup_target_path(&volume.target_path, volume.access_type)?;
        if let Some(staging_target_path) = &volume.staging_target_path {
            let operator = self.operator.clone();
            let uds_path = uds_path.clone();
            let volume_id = volume.volume_id.clone();
            let staging_target_path = staging_target_path.clone();
            let staging_target_path_for_rpc = staging_target_path.clone();
            let mount_namespace_path = volume.mount_namespace_path.clone();
            match run_in_mount_namespace(mount_namespace_path, move || async move {
                operator
                    .unstage(&uds_path, volume_id, staging_target_path_for_rpc)
                    .await
            })
            .await
            {
                Ok(()) | Err(CsiError::Driver(tugboat_csi_operator::Error::TargetPathNotFound)) => {
                }
                Err(err) => return Err(err),
            }
            cleanup_directory_path(staging_target_path)?;
        }
        if volume.controller_published {
            match self
                .operator
                .controller_unpublish(
                    &uds_path,
                    volume.volume_id.clone(),
                    node_name.to_string(),
                    controller_publish_secrets.clone(),
                )
                .await
            {
                Ok(()) | Err(tugboat_csi_operator::Error::VolumeNotFound) => {}
                Err(err) => return Err(CsiError::Driver(err)),
            }
        }
        self.remove_published_volume_state(volume)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn expand(
        &self,
        volume: &PublishedVolume,
        source: &CsiPersistentVolumeSource,
        claim: &PersistentVolumeClaimSpec,
        volume_spec: &PersistentVolumeSpec,
        secrets: &ResolvedCsiSecrets,
        capacity_bytes: i64,
    ) -> Result<Option<i64>, CsiError> {
        let Some(uds_path) = self.drivers.get(&volume.driver) else {
            return Err(CsiError::DriverNotFound(volume.driver.clone()));
        };
        let node_capabilities = self.operator.node_capabilities(uds_path).await?;
        if !node_capabilities.contains(&NodeCapability::ExpandVolume) {
            return Ok(None);
        }

        let (access_mode, _) = effective_publish_settings(
            &claim.access_modes,
            &volume_spec.access_modes,
            source.read_only,
        )?;
        let access_type = access_type_from_volume_mode(volume_spec.volume_mode.as_deref())?;
        let operator = self.operator.clone();
        let uds_path = uds_path.to_string();
        let volume_id = volume.volume_id.clone();
        let volume_path = volume.target_path.clone();
        let staging_target_path = volume.staging_target_path.clone();
        let mount_namespace_path = volume.mount_namespace_path.clone();
        let fs_type = filesystem_type(source, access_type);
        let node_expand_secrets = secrets.node_expand.clone();
        Ok(Some(
            run_in_mount_namespace(mount_namespace_path, move || async move {
                operator
                    .node_expand(
                        &uds_path,
                        volume_id,
                        volume_path,
                        capacity_bytes,
                        staging_target_path,
                        access_mode,
                        access_type,
                        fs_type,
                        node_expand_secrets,
                    )
                    .await
            })
            .await?,
        ))
    }

    pub(crate) async fn volume_stats(
        &self,
        volume: &PublishedVolume,
    ) -> Result<Option<NodeVolumeStats>, CsiError> {
        let Some(uds_path) = self.drivers.get(&volume.driver) else {
            return Err(CsiError::DriverNotFound(volume.driver.clone()));
        };
        let node_capabilities = self.operator.node_capabilities(uds_path).await?;
        if !node_capabilities.contains(&NodeCapability::GetVolumeStats) {
            return Ok(None);
        }

        let operator = self.operator.clone();
        let uds_path = uds_path.to_string();
        let volume_id = volume.volume_id.clone();
        let volume_path = volume.target_path.clone();
        let staging_target_path = volume.staging_target_path.clone();
        let mount_namespace_path = volume.mount_namespace_path.clone();
        Ok(Some(
            run_in_mount_namespace(mount_namespace_path, move || async move {
                operator
                    .node_volume_stats(&uds_path, volume_id, volume_path, staging_target_path)
                    .await
            })
            .await?,
        ))
    }

    pub(crate) async fn driver_requires_staging(&self, driver: &str) -> Result<bool, CsiError> {
        let Some(uds_path) = self.drivers.get(driver) else {
            return Err(CsiError::DriverNotFound(driver.to_string()));
        };
        let node_capabilities = self.operator.node_capabilities(uds_path).await?;
        Ok(node_capabilities.contains(&NodeCapability::StageUnstageVolume))
    }

    async fn rollback_published_volume(
        &self,
        node_name: &str,
        published: &PublishedVolume,
        controller_publish_secrets: &HashMap<String, String>,
        context: &str,
    ) {
        if let Err(cleanup_err) = self
            .unpublish(published, node_name, controller_publish_secrets)
            .await
        {
            tracing::error!("Failed to roll back {context}: {cleanup_err}");
        }
    }

    fn target_path(
        &self,
        ship_id: &str,
        claim_name: &str,
        access_type: PublishedAccessType,
    ) -> String {
        self.publish_dir
            .join(ship_id)
            .join(match access_type {
                PublishedAccessType::Block => format!("{claim_name}.block"),
                PublishedAccessType::Filesystem => format!("{claim_name}.fs"),
            })
            .display()
            .to_string()
    }

    fn staging_target_path(&self, ship_id: &str, claim_name: &str) -> String {
        self.publish_dir
            .join(ship_id)
            .join(".staging")
            .join(claim_name)
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

fn prepare_target_path(
    target_path: &str,
    access_type: PublishedAccessType,
) -> Result<(), CsiError> {
    let path = Path::new(target_path);
    let Some(parent) = path.parent() else {
        return Err(CsiError::TargetPathHasNoParent(target_path.to_string()));
    };
    create_dir_all(parent)?;
    match access_type {
        PublishedAccessType::Block => {
            // Directly attempt the file operation instead of a separate is_dir() check
            // to avoid a TOCTOU race between the check and the open.
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)?;
        }
        PublishedAccessType::Filesystem => prepare_directory_path(target_path)?,
    }
    Ok(())
}

fn prepare_directory_path(target_path: &str) -> Result<(), CsiError> {
    let path = Path::new(target_path);
    // Skip the separate is_file() check to avoid a TOCTOU race between the
    // check and create_dir_all. If the path is a regular file, create_dir_all
    // will return an appropriate IO error (ENOTDIR).
    create_dir_all(path)?;
    Ok(())
}

fn cleanup_target_path(
    target_path: &str,
    access_type: PublishedAccessType,
) -> Result<(), CsiError> {
    match access_type {
        PublishedAccessType::Block => {
            let path = Path::new(target_path);
            match remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        PublishedAccessType::Filesystem => cleanup_directory_path(target_path)?,
    }
    cleanup_target_parent_dir(target_path)?;
    Ok(())
}

fn cleanup_directory_path(target_path: impl AsRef<str>) -> Result<(), CsiError> {
    let path = Path::new(target_path.as_ref());
    match remove_dir(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    cleanup_target_parent_dir(target_path.as_ref())
}

fn cleanup_target_parent_dir(target_path: &str) -> Result<(), CsiError> {
    let path = Path::new(target_path);
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

pub(crate) fn access_type_from_volume_mode(mode: Option<&str>) -> Result<CsiAccessType, CsiError> {
    CsiAccessType::try_convert_from_string(mode.unwrap_or("Block"))
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

fn filesystem_type(
    source: &CsiPersistentVolumeSource,
    access_type: CsiAccessType,
) -> Option<String> {
    if !matches!(access_type, CsiAccessType::Filesystem) {
        return None;
    }
    source.fs_type.clone().filter(|fs_type| !fs_type.is_empty())
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

#[cfg(test)]
mod tests;
