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

pub(crate) mod state_manager;
pub(crate) mod types;

#[cfg(test)]
mod tests_failure_scenarios;

use crate::mountns;
use nix::sched::{CloneFlags, setns};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{OpenOptions, create_dir_all, read_dir, remove_dir, remove_file};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::time::{Duration, sleep};
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

use state_manager::StateManager;

const CSI_RETRY_MAX_ATTEMPTS: usize = 3;

#[cfg(not(test))]
const CSI_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);

#[cfg(test)]
const CSI_RETRY_BASE_DELAY: Duration = Duration::from_millis(10);

#[derive(Clone)]
pub(crate) struct CsiWrapper {
    operator: TugboatCsiOperator,
    drivers: CsiDrivers,
    publish_dir: PathBuf,
    state_manager: Arc<StateManager>,
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
            state_manager: Arc::new(StateManager::new()),
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

    pub(crate) async fn load_published_volumes(
        &self,
        ship_id: &str,
    ) -> Result<Vec<PublishedVolume>, CsiError> {
        let state_manager = self.state_manager.clone();
        let publish_dir = self.publish_dir.clone();
        let ship_id_str = ship_id.to_string();

        state_manager
            .with_lock(ship_id, move || {
                let ship_dir = publish_dir.join(&ship_id_str);
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
            })
            .await
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
        let volume_id = source.volume_handle.clone();
        let node_capabilities = self
            .retry_csi_operation("load node capabilities", &volume_id, || {
                let operator = self.operator.clone();
                let uds_path = uds_path.to_string();
                async move {
                    operator
                        .node_capabilities(&uds_path)
                        .await
                        .map_err(CsiError::from)
                }
            })
            .await?;
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
        let controller_capabilities = self
            .retry_csi_operation("load controller capabilities", &volume_id, || {
                let operator = self.operator.clone();
                let uds_path = uds_path.to_string();
                async move {
                    operator
                        .controller_capabilities(&uds_path)
                        .await
                        .map_err(CsiError::from)
                }
            })
            .await?;
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
                self.retry_csi_operation("controller publish volume", &published.volume_id, || {
                    let operator = self.operator.clone();
                    let uds_path = uds_path.to_string();
                    let volume_id = published.volume_id.clone();
                    let node_name = node_name.to_string();
                    let fs_type = fs_type.clone();
                    let controller_publish_secrets = secrets.controller_publish.clone();
                    let volume_context = volume_context.clone();
                    async move {
                        operator
                            .controller_publish(
                                &uds_path,
                                volume_id,
                                node_name,
                                read_only,
                                access_mode,
                                access_type,
                                fs_type,
                                controller_publish_secrets,
                                volume_context,
                            )
                            .await
                            .map_err(CsiError::from)
                    }
                })
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
            match self
                .retry_csi_operation("stage volume", &published.volume_id, || {
                    let operator = operator.clone();
                    let uds_path = uds_path.clone();
                    let volume_id = volume_id.clone();
                    let staging_target_path_for_rpc = staging_target_path_for_rpc.clone();
                    let mount_namespace_path = mount_namespace_path.clone();
                    let node_stage_secrets = node_stage_secrets.clone();
                    let fs_type = fs_type.clone();
                    let mount_flags = mount_flags.clone();
                    let volume_context = volume_context.clone();
                    let publish_context = publish_context.clone();
                    async move {
                        run_in_mount_namespace(mount_namespace_path, move || async move {
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
                    }
                })
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    let context = "controller-published volume after stage failure";
                    if published.controller_published {
                        if let Err(rollback_err) = self
                            .retry_csi_operation(
                                "rollback controller unpublish after stage failure",
                                &published.volume_id,
                                || {
                                    let operator = self.operator.clone();
                                    let uds_path_for_controller = uds_path_for_controller.clone();
                                    let volume_id = published.volume_id.clone();
                                    let node_name = node_name.to_string();
                                    let controller_publish_secrets =
                                        secrets.controller_publish.clone();
                                    async move {
                                        operator
                                            .controller_unpublish(
                                                &uds_path_for_controller,
                                                volume_id,
                                                node_name,
                                                controller_publish_secrets,
                                            )
                                            .await
                                            .map_err(CsiError::from)
                                    }
                                },
                            )
                            .await
                        {
                            let _ = cleanup_directory_path(staging_target_path);
                            return Err(CsiError::RollbackFailed {
                                context: context.to_string(),
                                original: err.to_string(),
                                rollback: rollback_err.to_string(),
                            });
                        }
                    }
                    let _ = cleanup_directory_path(staging_target_path);
                    return Err(err);
                }
            }
            staged = true;
        }
        if let Err(err) = prepare_target_path(&published.target_path, published.access_type) {
            if staged {
                let context = "staged volume after target-path preparation error";
                if let Err(rollback_err) = self
                    .rollback_published_volume(
                        node_name,
                        &published,
                        &secrets.controller_publish,
                        context,
                    )
                    .await
                {
                    return Err(CsiError::RollbackFailed {
                        context: context.to_string(),
                        original: err.to_string(),
                        rollback: rollback_err.to_string(),
                    });
                }
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
        if let Err(err) = self
            .retry_csi_operation("publish volume", &published.volume_id, || {
                let operator = operator.clone();
                let uds_path = uds_path.clone();
                let volume_id = volume_id.clone();
                let target_path = target_path.clone();
                let mount_namespace_path = mount_namespace_path.clone();
                let staging_target_path = staging_target_path.clone();
                let node_publish_secrets = node_publish_secrets.clone();
                let fs_type = fs_type.clone();
                let mount_flags = mount_flags.clone();
                let volume_context = volume_context.clone();
                let publish_context = publish_context.clone();
                async move {
                    run_in_mount_namespace(mount_namespace_path, move || async move {
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
                }
            })
            .await
        {
            let context = "staged/published volume after publish error";
            if let Err(rollback_err) = self
                .rollback_published_volume(
                    node_name,
                    &published,
                    &secrets.controller_publish,
                    context,
                )
                .await
            {
                return Err(CsiError::RollbackFailed {
                    context: context.to_string(),
                    original: err.to_string(),
                    rollback: rollback_err.to_string(),
                });
            }
            return Err(err);
        }
        if let Err(err) = self.persist_published_volume(&published, ship_id).await {
            // CRITICAL: Volume is mounted on node but state file write failed.
            // DO NOT rollback - rollback might also fail and leave volume in inconsistent state.
            // Instead, return a typed error indicating the volume is partially published.
            // Caller can retry persist or decide on cleanup strategy.
            tracing::error!(
                "Failed to persist CSI published volume state for '{}' (volume IS mounted on node): {}. \
                 Volume will be recoverable on next reconciliation attempt.",
                published.volume_id,
                err
            );
            return Err(CsiError::PublishPartialState {
                volume_id: published.volume_id.clone(),
                reason: err.to_string(),
            });
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
        match self
            .retry_csi_operation("unpublish volume", &volume.volume_id, || {
                let operator = operator.clone();
                let uds_path_for_unpublish = uds_path_for_unpublish.clone();
                let volume_id = volume_id.clone();
                let target_path = target_path.clone();
                let mount_namespace_path = mount_namespace_path.clone();
                async move {
                    run_in_mount_namespace(mount_namespace_path, move || async move {
                        operator
                            .unpublish(&uds_path_for_unpublish, volume_id, target_path)
                            .await
                    })
                    .await
                }
            })
            .await
        {
            Ok(()) => {}
            Err(CsiError::Driver(tugboat_csi_operator::Error::TargetPathNotFound)) => {
                tracing::warn!(
                    "CSI reported target path not found during unpublish (volume_id='{}', path='{}'); treating as idempotent cleanup",
                    volume.volume_id,
                    volume.target_path
                );
            }
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
            match self
                .retry_csi_operation("unstage volume", &volume.volume_id, || {
                    let operator = operator.clone();
                    let uds_path = uds_path.clone();
                    let volume_id = volume_id.clone();
                    let staging_target_path_for_rpc = staging_target_path_for_rpc.clone();
                    let mount_namespace_path = mount_namespace_path.clone();
                    async move {
                        run_in_mount_namespace(mount_namespace_path, move || async move {
                            operator
                                .unstage(&uds_path, volume_id, staging_target_path_for_rpc)
                                .await
                        })
                        .await
                    }
                })
                .await
            {
                Ok(()) => {}
                Err(CsiError::Driver(tugboat_csi_operator::Error::TargetPathNotFound)) => {
                    tracing::warn!(
                        "CSI reported staging path not found during unstage (volume_id='{}', staging_path='{}'); treating as idempotent cleanup",
                        volume.volume_id,
                        staging_target_path
                    );
                }
                Err(err) => return Err(err),
            }
            cleanup_directory_path(staging_target_path)?;
        }
        if volume.controller_published {
            match self
                .retry_csi_operation("controller unpublish volume", &volume.volume_id, || {
                    let operator = self.operator.clone();
                    let uds_path = uds_path.to_string();
                    let volume_id = volume.volume_id.clone();
                    let node_name = node_name.to_string();
                    let controller_publish_secrets = controller_publish_secrets.clone();
                    async move {
                        operator
                            .controller_unpublish(
                                &uds_path,
                                volume_id,
                                node_name,
                                controller_publish_secrets,
                            )
                            .await
                            .map_err(CsiError::from)
                    }
                })
                .await
            {
                Ok(()) | Err(CsiError::Driver(tugboat_csi_operator::Error::VolumeNotFound)) => {}
                Err(err) => return Err(err),
            }
        }
        self.remove_published_volume_state(volume).await?;
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
        let node_capabilities = self
            .retry_csi_operation("load node capabilities", &volume.volume_id, || {
                let operator = self.operator.clone();
                let uds_path = uds_path.to_string();
                async move {
                    operator
                        .node_capabilities(&uds_path)
                        .await
                        .map_err(CsiError::from)
                }
            })
            .await?;
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
            self.retry_csi_operation("expand volume", &volume.volume_id, || {
                let operator = operator.clone();
                let uds_path = uds_path.clone();
                let volume_id = volume_id.clone();
                let volume_path = volume_path.clone();
                let staging_target_path = staging_target_path.clone();
                let mount_namespace_path = mount_namespace_path.clone();
                let fs_type = fs_type.clone();
                let node_expand_secrets = node_expand_secrets.clone();
                async move {
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
                    .await
                }
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
        let node_capabilities = self
            .retry_csi_operation("load node capabilities", &volume.volume_id, || {
                let operator = self.operator.clone();
                let uds_path = uds_path.to_string();
                async move {
                    operator
                        .node_capabilities(&uds_path)
                        .await
                        .map_err(CsiError::from)
                }
            })
            .await?;
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
            self.retry_csi_operation("fetch volume stats", &volume.volume_id, || {
                let operator = operator.clone();
                let uds_path = uds_path.clone();
                let volume_id = volume_id.clone();
                let volume_path = volume_path.clone();
                let staging_target_path = staging_target_path.clone();
                let mount_namespace_path = mount_namespace_path.clone();
                async move {
                    run_in_mount_namespace(mount_namespace_path, move || async move {
                        operator
                            .node_volume_stats(
                                &uds_path,
                                volume_id,
                                volume_path,
                                staging_target_path,
                            )
                            .await
                    })
                    .await
                }
            })
            .await?,
        ))
    }

    pub(crate) async fn driver_requires_staging(&self, driver: &str) -> Result<bool, CsiError> {
        let Some(uds_path) = self.drivers.get(driver) else {
            return Err(CsiError::DriverNotFound(driver.to_string()));
        };
        let node_capabilities = self
            .retry_csi_operation("load node capabilities", driver, || {
                let operator = self.operator.clone();
                let uds_path = uds_path.to_string();
                async move {
                    operator
                        .node_capabilities(&uds_path)
                        .await
                        .map_err(CsiError::from)
                }
            })
            .await?;
        Ok(node_capabilities.contains(&NodeCapability::StageUnstageVolume))
    }

    pub(crate) async fn recover_partial_published_volume_state(
        &self,
        ship_id: &str,
        planned: &[PublishedVolume],
    ) -> Result<Option<Vec<PublishedVolume>>, CsiError> {
        if planned.is_empty() {
            return Ok(None);
        }
        let mut write_plan = Vec::with_capacity(planned.len());
        for volume in planned {
            write_plan.push((self.state_path_for_volume(volume)?, volume.clone()));
        }

        let state_manager = self.state_manager.clone();
        state_manager
            .with_lock(ship_id, move || {
                if !write_plan
                    .iter()
                    .all(|(_, volume)| Self::looks_like_partially_published_volume(volume))
                {
                    return Ok(None);
                }

                for (path, volume) in &write_plan {
                    state_manager::atomic_write_json(path, volume)?;
                }

                let mut recovered = write_plan
                    .into_iter()
                    .map(|(_, volume)| volume)
                    .collect::<Vec<_>>();
                recovered.sort_by(|left, right| left.target_path.cmp(&right.target_path));
                Ok(Some(recovered))
            })
            .await
    }

    async fn rollback_published_volume(
        &self,
        node_name: &str,
        published: &PublishedVolume,
        controller_publish_secrets: &HashMap<String, String>,
        context: &str,
    ) -> Result<(), CsiError> {
        self.unpublish(published, node_name, controller_publish_secrets)
            .await
            .map_err(|cleanup_err| {
                tracing::error!("Failed to roll back {context}: {cleanup_err}");
                cleanup_err
            })
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

    async fn persist_published_volume(
        &self,
        volume: &PublishedVolume,
        ship_id: &str,
    ) -> Result<(), CsiError> {
        let path = self.state_path_for_volume(volume)?;
        let volume_copy = volume.clone();
        let state_manager = self.state_manager.clone();

        state_manager
            .with_lock(ship_id, move || {
                state_manager::atomic_write_json(&path, &volume_copy)
            })
            .await
    }

    async fn remove_published_volume_state(
        &self,
        volume: &PublishedVolume,
    ) -> Result<(), CsiError> {
        let ship_id = volume.extract_ship_id().ok_or_else(|| {
            CsiError::TargetPathHasNoParent(format!(
                "Could not extract ship_id from mount namespace path: {}",
                volume.mount_namespace_path
            ))
        })?;

        let path = self.state_path_for_volume(volume)?;
        let state_manager = self.state_manager.clone();

        state_manager
            .with_lock(ship_id, move || {
                state_manager::safe_remove_file(&path)?;
                if let Some(parent) = path.parent() {
                    state_manager::safe_remove_dir(parent)?;
                }
                Ok(())
            })
            .await
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

    fn looks_like_partially_published_volume(volume: &PublishedVolume) -> bool {
        let target_path = Path::new(&volume.target_path);
        let target_exists = match volume.access_type {
            PublishedAccessType::Block => target_path.is_file(),
            PublishedAccessType::Filesystem => target_path.is_dir(),
        };
        if !target_exists {
            return false;
        }
        if !Path::new(&volume.mount_namespace_path).exists() {
            return false;
        }

        volume
            .staging_target_path
            .as_ref()
            .is_none_or(|path| Path::new(path).exists())
    }

    async fn retry_csi_operation<T, F, Fut>(
        &self,
        operation_name: &str,
        volume_id: &str,
        mut operation: F,
    ) -> Result<T, CsiError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, CsiError>>,
    {
        for attempt in 1..=CSI_RETRY_MAX_ATTEMPTS {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(err) if attempt < CSI_RETRY_MAX_ATTEMPTS && is_retryable_csi_error(&err) => {
                    let delay = CSI_RETRY_BASE_DELAY.saturating_mul(2u32.pow((attempt - 1) as u32));
                    tracing::warn!(
                        "Transient CSI failure during {} for '{}' (attempt {}/{}): {}",
                        operation_name,
                        volume_id,
                        attempt,
                        CSI_RETRY_MAX_ATTEMPTS,
                        err
                    );
                    sleep(delay).await;
                }
                Err(err) => return Err(err),
            }
        }

        unreachable!("CSI retry loop should return before exhausting attempts");
    }
}

fn is_retryable_csi_error(error: &CsiError) -> bool {
    match error {
        CsiError::Driver(inner) => is_retryable_driver_error(inner),
        _ => false,
    }
}

fn is_retryable_driver_error(error: &tugboat_csi_operator::Error) -> bool {
    match error {
        tugboat_csi_operator::Error::RpcTimeout
        | tugboat_csi_operator::Error::SocketConnectionTimeout
        | tugboat_csi_operator::Error::GrpcTransport(_) => true,
        tugboat_csi_operator::Error::Grpc(status) => matches!(
            status.code(),
            tonic::Code::Unavailable
                | tonic::Code::DeadlineExceeded
                | tonic::Code::Aborted
                | tonic::Code::ResourceExhausted
                | tonic::Code::Unknown
                | tonic::Code::Internal
        ),
        _ => false,
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
