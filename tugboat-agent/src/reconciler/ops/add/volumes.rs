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

use crate::csi::{
    PublishedAccessType, PublishedVolume, access_type_from_volume_mode, effective_publish_settings,
};
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::ops::add_helpers::vm_volume_config;
use crate::reconciler::volume::VolumeInfo;
use std::collections::HashMap;
use tracing::{error, warn};
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

use super::recovery::find_recovered_published_volume;

pub(super) struct VolumeSetupGuard<'a> {
    reconciler: &'a ShipReconciler,
    pub(super) ship_id: &'a str,
    pub(super) published_volumes: Vec<PublishedVolume>,
    pub(super) controller_publish_secrets: HashMap<String, HashMap<String, String>>,
}

impl<'a> VolumeSetupGuard<'a> {
    pub(super) fn new(reconciler: &'a ShipReconciler, ship_id: &'a str) -> Self {
        Self {
            reconciler,
            ship_id,
            published_volumes: Vec::new(),
            controller_publish_secrets: HashMap::new(),
        }
    }

    pub(super) async fn cancel(self, context: &str) -> Vec<String> {
        let cleanup_errors = self
            .reconciler
            .cleanup_after_volume_setup_error(
                self.ship_id,
                &self.published_volumes,
                &self.controller_publish_secrets,
                context,
            )
            .await;
        if !cleanup_errors.is_empty() {
            error!(
                "Cleanup after {} for ship '{}' completed with errors: {}",
                context,
                self.ship_id,
                cleanup_errors.join("; ")
            );
        }
        cleanup_errors
    }

    pub(super) fn commit(self) -> Vec<PublishedVolume> {
        self.published_volumes
    }
}

impl ShipReconciler {
    pub(crate) async fn setup_volumes(
        &self,
        ship_id: &str,
        namespace: &str,
        volumes: &[VolumeInfo],
    ) -> Result<(Vec<PublishedVolume>, Vec<VmVolumeConfig>), ReconcileError> {
        let mut guard = VolumeSetupGuard::new(self, ship_id);

        let vm_volumes = self
            .setup_volumes_inner(&mut guard, namespace, volumes)
            .await;

        match vm_volumes {
            Ok(vm_volumes) => Ok((guard.commit(), vm_volumes)),
            Err(err) => {
                let cleanup_errors = guard.cancel("volume setup error").await;
                if !cleanup_errors.is_empty() {
                    return Err(ReconcileError::VolumeSetupCleanupFailed {
                        original_error: Box::new(err),
                        cleanup_errors: cleanup_errors.join("; "),
                    });
                }
                Err(err)
            }
        }
    }

    async fn setup_volumes_inner(
        &self,
        guard: &mut VolumeSetupGuard<'_>,
        namespace: &str,
        volumes: &[VolumeInfo],
    ) -> Result<Vec<VmVolumeConfig>, ReconcileError> {
        let mut vm_volumes = Vec::new();
        if volumes
            .iter()
            .any(|volume| volume.persistent_volume_claim().is_some())
        {
            self.csi.ensure_mount_namespace(guard.ship_id)?;
        }

        for volume in volumes {
            if let Some(volume) = volume.persistent_volume_claim() {
                let (_, read_only) = effective_publish_settings(
                    &volume.claim.access_modes,
                    &volume.volume.access_modes,
                    volume.source.read_only,
                )?;
                let secrets = self.resolve_csi_secrets(volume).await?;

                let published = match self
                    .csi
                    .publish(
                        &self.node_name,
                        guard.ship_id,
                        &volume.name,
                        &volume.claim_name,
                        &volume.volume,
                        &volume.claim,
                        &volume.source,
                        &secrets,
                    )
                    .await
                {
                    Ok(published) => published,
                    Err(crate::csi::CsiError::PublishPartialState {
                        volume_id,
                        reason,
                        published,
                    }) => {
                        guard
                            .controller_publish_secrets
                            .insert(volume.name.clone(), secrets.controller_publish.clone());
                        guard.published_volumes.push(*published);

                        // Volume is mounted but state file write failed (disk full, permission denied, etc).
                        // Log this as a recoverable error - the volume IS accessible on the node.
                        // Next reconciliation will detect it and complete the setup.
                        warn!(
                            "CSI volume '{}' (claim='{}', ship_id='{}') is mounted on node but state persistence failed ({}). \
                             Ship will be marked as failed; retry will recover and persist state.",
                            volume_id, volume.claim_name, guard.ship_id, reason
                        );
                        return Err(ReconcileError::CsiVolumePartiallyPublished {
                            volume_id,
                            reason,
                        });
                    }
                    Err(err) => return Err(err.into()),
                };

                guard
                    .controller_publish_secrets
                    .insert(volume.name.clone(), secrets.controller_publish.clone());
                guard.published_volumes.push(published.clone());

                self.ensure_node_expansion(namespace, volume, &published, &secrets)
                    .await?;
                self.refresh_volume_stats(namespace, volume, &published)
                    .await?;
                self.mark_volume_attached(&volume.volume_name, true).await?;

                vm_volumes.push(vm_volume_config(volume, &published, read_only));
            } else if let Some(volume) = volume.materialized() {
                let path = self.materialize_volume(guard.ship_id, volume)?;
                self.start_service_account_token_refresh(guard.ship_id, namespace, volume);
                vm_volumes.push(VmVolumeConfig::filesystem(path, volume.name.clone(), true));
            }
        }
        Ok(vm_volumes)
    }

    pub(super) async fn plan_desired_published_volumes(
        &self,
        ship_id: &str,
        volumes: &[VolumeInfo],
    ) -> Result<Vec<PublishedVolume>, ReconcileError> {
        let mut planned = Vec::with_capacity(volumes.len());
        for volume in volumes {
            let Some(volume) = volume.persistent_volume_claim() else {
                continue;
            };
            let access_type = PublishedAccessType::from(access_type_from_volume_mode(
                volume.volume.volume_mode.as_deref(),
            )?);
            let requires_staging = self
                .csi
                .driver_requires_staging(&volume.source.driver)
                .await?;
            planned.push(self.csi.plan_published_volume(
                ship_id,
                &volume.name,
                &volume.claim_name,
                &volume.source,
                access_type,
                requires_staging,
            )?);
        }
        Ok(planned)
    }

    pub(super) async fn setup_recovered_published_volumes(
        &self,
        ship_id: &str,
        namespace: &str,
        volumes: &[VolumeInfo],
        recovered_published_volumes: &[PublishedVolume],
    ) -> Result<Vec<VmVolumeConfig>, ReconcileError> {
        let mut vm_volumes = Vec::new();
        if volumes
            .iter()
            .any(|volume| volume.persistent_volume_claim().is_some())
        {
            self.csi.ensure_mount_namespace(ship_id)?;
        }

        for volume in volumes {
            if let Some(volume) = volume.persistent_volume_claim() {
                let Some(published) = find_recovered_published_volume(
                    recovered_published_volumes,
                    &volume.name,
                    &volume.source.volume_handle,
                ) else {
                    return Err(ReconcileError::RecoveredPublishedVolumeStateMismatch(
                        ship_id.to_string(),
                    ));
                };

                let (_, read_only) = effective_publish_settings(
                    &volume.claim.access_modes,
                    &volume.volume.access_modes,
                    volume.source.read_only,
                )?;
                let secrets = self.resolve_csi_secrets(volume).await?;

                self.ensure_node_expansion(namespace, volume, published, &secrets)
                    .await?;
                self.refresh_volume_stats(namespace, volume, published)
                    .await?;
                self.mark_volume_attached(&volume.volume_name, true).await?;

                vm_volumes.push(vm_volume_config(volume, published, read_only));
            } else if let Some(volume) = volume.materialized() {
                let path = self.materialize_volume(ship_id, volume)?;
                self.start_service_account_token_refresh(ship_id, namespace, volume);
                vm_volumes.push(VmVolumeConfig::filesystem(path, volume.name.clone(), true));
            }
        }

        Ok(vm_volumes)
    }
}
