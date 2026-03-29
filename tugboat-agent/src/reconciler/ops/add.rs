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
use crate::reconciler::ops::add_helpers::{
    apply_persistent_volume_claim_csi_observation, apply_persistent_volume_csi_observation,
    validate_recovered_published_volumes, vm_volume_config,
};
use crate::reconciler::reconcile::AppendStatus;
use crate::reconciler::volume::{PersistentVolumeClaimVolumeInfo, VolumeInfo};
use crate::runtime::RuntimeCreateRequest;
use std::collections::HashMap;
use tracing::{debug, error, info};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimStatus, Ship, ShipCondition,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

impl ShipReconciler {
    pub(crate) async fn reconcile_added(&self, ship: Ship) -> Result<(), ReconcileError> {
        info!("Starting reconciliation for ship");
        debug!("Checking ship configuration");
        let Some(ship_metadata) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(name) = &ship_metadata.name else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let Some(ship_id) = &ship_metadata.uid else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.uid".to_string(),
            ));
        };
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };
        debug!("Getting ship class named '{}'", ship_spec.ship_class);
        let Some(class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
            return Err(ReconcileError::ShipClassNotFound(
                ship_spec.ship_class.clone(),
            ));
        };
        let namespace = ship_metadata
            .namespace
            .clone()
            .unwrap_or("default".to_string());
        let spec_fp = super::spec_fingerprint(ship_spec)?;
        let volume_fp = super::volume_claims_fingerprint(ship_spec)?;

        if self.runtime_operator.is_present(ship_id).await? {
            let volumes = self.get_related_volumes(&namespace, ship_spec).await?;
            let planned_published_volumes = self
                .plan_desired_published_volumes(ship_id, &volumes)
                .await?;
            let published_volumes = validate_recovered_published_volumes(
                ship_id,
                self.csi.load_published_volumes(ship_id)?,
                &planned_published_volumes,
            )?;
            self.runtime_operator
                .register_existing(
                    namespace,
                    name.clone(),
                    ship_id.clone(),
                    spec_fp,
                    volume_fp,
                    published_volumes,
                )
                .await;
            info!("Recovered existing VM runtime state for ship '{}'", ship_id);
            return Ok(());
        }

        debug!("Getting network classes for ship");
        let network_classes = self
            .get_related_network_classes(&namespace, ship_spec)
            .await?;
        debug!("{} network classes loaded", network_classes.len());
        debug!("Getting volume claims for ship");
        let volumes = self.get_related_volumes(&namespace, ship_spec).await?;
        debug!("{} volumes loaded", volumes.len());
        let stale_published_volumes = self.csi.load_published_volumes(ship_id)?;
        if !stale_published_volumes.is_empty() {
            info!("Cleaning up stale CSI publish state for ship '{}'", ship_id);
            let controller_publish_secrets = self.controller_publish_secret_map(&volumes).await?;
            self.cleanup_published_volumes(&stale_published_volumes, &controller_publish_secrets)
                .await?;
            self.csi.cleanup_mount_namespace(ship_id)?;
        }
        self.cleanup_materialized_volumes(ship_id)?;

        {
            debug!("Updating Ship status");
            let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
            let mut status_ship = ship.clone();
            status_ship.append_status(ShipCondition {
                status: "VmCreating".to_string(),
                message: "Creating new Virtual Machine".to_string(),
                timestamp: Some(Time::now()),
            });
            api.replace_status(name, status_ship).await?;
        }

        debug!("Planning network configurations for ship");
        let networks = self.cni.create_network_configs(ship_id, network_classes);
        let (published_volumes, vm_volumes) =
            self.setup_volumes(ship_id, &namespace, &volumes).await?;

        self.with_cleanup(ship_id, published_volumes.as_slice(), &volumes, || {
            let volumes = vm_volumes;
            let published_volumes = published_volumes.clone();
            async move {
                debug!("Setup virtual machine");
                if let Err(err) = self
                    .runtime_operator
                    .create(RuntimeCreateRequest {
                        ship_id: ship_id.clone(),
                        ship_name: name.clone(),
                        namespace,
                        ship_spec,
                        ship_class: class,
                        networks: networks.iter().map(|n| n.vm.clone()).collect(),
                        volumes,
                        spec_fingerprint: spec_fp,
                        volume_fingerprint: volume_fp,
                        published_volumes,
                    })
                    .await
                {
                    return Err(err.into());
                }
                debug!("Creating network resources");
                if let Err(err) = self.cni.add(ship_id, networks).await {
                    return Err(err.into());
                }
                debug!("Starting runtime operator");
                if let Err(err) = self.runtime_operator.start(ship_id).await {
                    return Err(err.into());
                }
                Ok(())
            }
        })
        .await
    }

    async fn setup_volumes(
        &self,
        ship_id: &str,
        namespace: &str,
        volumes: &[VolumeInfo],
    ) -> Result<(Vec<PublishedVolume>, Vec<VmVolumeConfig>), ReconcileError> {
        let mut published_volumes = Vec::new();
        let mut vm_volumes = Vec::new();
        let mut controller_publish_secrets = HashMap::new();
        if volumes
            .iter()
            .any(|volume| volume.persistent_volume_claim().is_some())
        {
            self.csi.ensure_mount_namespace(ship_id)?;
        }
        for volume in volumes {
            if let Some(volume) = volume.persistent_volume_claim() {
                let (_, read_only) = effective_publish_settings(
                    &volume.claim.access_modes,
                    &volume.volume.access_modes,
                    volume.source.read_only,
                )?;
                let secrets = match self.resolve_csi_secrets(volume).await {
                    Ok(secrets) => secrets,
                    Err(err) => {
                        self.cleanup_after_volume_setup_error(
                            ship_id,
                            &published_volumes,
                            &controller_publish_secrets,
                            "secret resolution error",
                        )
                        .await;
                        return Err(err);
                    }
                };
                match self
                    .csi
                    .publish(
                        &self.node_name,
                        ship_id,
                        &volume.name,
                        &volume.volume,
                        &volume.claim,
                        &volume.source,
                        &secrets,
                    )
                    .await
                {
                    Ok(published) => {
                        controller_publish_secrets
                            .insert(volume.name.clone(), secrets.controller_publish.clone());
                        let mut cleanup_targets = published_volumes.clone();
                        cleanup_targets.push(published.clone());
                        if let Err(err) = self
                            .ensure_node_expansion(namespace, volume, &published, &secrets)
                            .await
                        {
                            self.cleanup_after_volume_setup_error(
                                ship_id,
                                &cleanup_targets,
                                &controller_publish_secrets,
                                "node expansion error",
                            )
                            .await;
                            return Err(err);
                        }
                        if let Err(err) = self
                            .refresh_volume_stats(namespace, volume, &published)
                            .await
                        {
                            self.cleanup_after_volume_setup_error(
                                ship_id,
                                &cleanup_targets,
                                &controller_publish_secrets,
                                "volume stats refresh error",
                            )
                            .await;
                            return Err(err);
                        }
                        if let Err(err) = self.mark_volume_attached(&volume.volume_name, true).await
                        {
                            self.cleanup_after_volume_setup_error(
                                ship_id,
                                &cleanup_targets,
                                &controller_publish_secrets,
                                "attachment status error",
                            )
                            .await;
                            return Err(err);
                        }
                        vm_volumes.push(vm_volume_config(volume, &published, read_only));
                        published_volumes.push(published);
                    }
                    Err(err) => {
                        self.cleanup_after_volume_setup_error(
                            ship_id,
                            &published_volumes,
                            &controller_publish_secrets,
                            "publish error",
                        )
                        .await;
                        return Err(err.into());
                    }
                }
            } else if let Some(volume) = volume.materialized() {
                match self.materialize_volume(ship_id, volume) {
                    Ok(path) => {
                        vm_volumes.push(VmVolumeConfig::filesystem(path, volume.name.clone(), true))
                    }
                    Err(err) => {
                        self.cleanup_after_volume_setup_error(
                            ship_id,
                            &published_volumes,
                            &controller_publish_secrets,
                            "materialization error",
                        )
                        .await;
                        return Err(err);
                    }
                };
            }
        }
        Ok((published_volumes, vm_volumes))
    }

    async fn cleanup_after_volume_setup_error(
        &self,
        ship_id: &str,
        cleanup_targets: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
        context: &str,
    ) {
        if let Err(cleanup_err) = self
            .cleanup_published_volumes(cleanup_targets, controller_publish_secrets)
            .await
        {
            error!("Failed to clean up published volumes after {context}: {cleanup_err}");
        }
        if let Err(cleanup_err) = self.cleanup_materialized_volumes(ship_id) {
            error!("Failed to clean up materialized volumes after {context}: {cleanup_err}");
        }
        if let Err(cleanup_err) = self.csi.cleanup_mount_namespace(ship_id) {
            error!("Failed to clean up mount namespace after {context}: {cleanup_err}");
        }
    }

    async fn plan_desired_published_volumes(
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
                &volume.source,
                access_type,
                requires_staging,
            )?);
        }
        Ok(planned)
    }

    async fn with_cleanup<Fut, F>(
        &self,
        ship_id: &str,
        published_volumes: &[PublishedVolume],
        volumes: &[VolumeInfo],
        f: F,
    ) -> Result<(), ReconcileError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), ReconcileError>>,
    {
        match f().await {
            Ok(_) => Ok(()),
            Err(e) => {
                if let Err(cleanup_err) = self
                    .cleanup_runtime_and_published_volumes(ship_id, published_volumes, volumes)
                    .await
                {
                    error!(
                        "Failed to clean up runtime and published volumes after start error: {cleanup_err}"
                    );
                }
                Err(e)
            }
        }
    }

    pub(crate) async fn cleanup_published_volumes(
        &self,
        published_volumes: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
    ) -> Result<(), ReconcileError> {
        let mut errors = Vec::new();
        for volume in published_volumes.iter().rev() {
            let secrets = controller_publish_secrets
                .get(&volume.claim_name)
                .cloned()
                .unwrap_or_default();
            if let Err(err) = self.csi.unpublish(volume, &self.node_name, &secrets).await {
                error!(
                    "Failed to unpublish CSI volume '{}' for ship mount namespace '{}': {err}",
                    volume.target_path, volume.mount_namespace_path
                );
                errors.push(format!("{}: {err}", volume.target_path));
            }
        }
        if !errors.is_empty() {
            return Err(ReconcileError::PublishedVolumeCleanupFailed(
                errors.join("; "),
            ));
        }
        Ok(())
    }

    async fn cleanup_runtime_and_published_volumes(
        &self,
        ship_id: &str,
        fallback_published_volumes: &[PublishedVolume],
        volumes: &[VolumeInfo],
    ) -> Result<(), ReconcileError> {
        let mut runtime_published_volumes =
            self.runtime_operator.delete(ship_id.to_string()).await?;
        let controller_publish_secrets = self.controller_publish_secret_map(volumes).await?;
        if runtime_published_volumes.is_empty() {
            runtime_published_volumes = self.csi.load_published_volumes(ship_id)?;
        }
        if runtime_published_volumes.is_empty() {
            self.cleanup_published_volumes(fallback_published_volumes, &controller_publish_secrets)
                .await?;
        } else {
            self.cleanup_published_volumes(&runtime_published_volumes, &controller_publish_secrets)
                .await?;
        }
        self.cleanup_materialized_volumes(ship_id)?;
        self.csi.cleanup_mount_namespace(ship_id)?;
        Ok(())
    }

    pub(super) async fn controller_publish_secret_map(
        &self,
        volumes: &[VolumeInfo],
    ) -> Result<HashMap<String, HashMap<String, String>>, ReconcileError> {
        let mut secrets = HashMap::with_capacity(volumes.len());
        for volume in volumes {
            let Some(volume) = volume.persistent_volume_claim() else {
                continue;
            };
            let resolved = self.resolve_csi_secrets(volume).await?;
            secrets.insert(volume.name.clone(), resolved.controller_publish);
        }
        Ok(secrets)
    }

    pub(super) async fn ensure_node_expansion(
        &self,
        namespace: &str,
        volume: &PersistentVolumeClaimVolumeInfo,
        published: &PublishedVolume,
        secrets: &crate::csi::ResolvedCsiSecrets,
    ) -> Result<(), ReconcileError> {
        let needs_node_expansion = volume
            .status
            .as_ref()
            .and_then(|status| status.node_expansion_required)
            .unwrap_or(false);
        if !needs_node_expansion {
            return Ok(());
        }
        let Some(target_capacity_bytes) = volume
            .status
            .as_ref()
            .and_then(|status| status.capacity_bytes)
            .or(volume.volume.capacity_bytes)
            .or(volume.claim.requested_capacity_bytes)
            .filter(|value| *value > 0)
        else {
            return Ok(());
        };
        let expanded_capacity_bytes = self
            .csi
            .expand(
                published,
                &volume.source,
                &volume.claim,
                &volume.volume,
                secrets,
                target_capacity_bytes,
            )
            .await?;
        let Some(expanded_capacity_bytes) = expanded_capacity_bytes else {
            return Err(ReconcileError::UnsupportedPersistentVolumeCsiFeature {
                volume: volume.volume_name.clone(),
                feature: "node_expand".to_string(),
            });
        };
        self.mark_volume_node_expanded(
            namespace,
            &volume.claim_name,
            &volume.volume_name,
            expanded_capacity_bytes,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn refresh_volume_stats(
        &self,
        namespace: &str,
        volume: &PersistentVolumeClaimVolumeInfo,
        published: &PublishedVolume,
    ) -> Result<(), ReconcileError> {
        let Some(stats) = self.csi.volume_stats(published).await? else {
            return Ok(());
        };

        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(mut persistent_volume) = pv_api.get(&volume.volume_name).await? else {
            return Ok(());
        };
        let mut persistent_volume_changed = false;
        {
            let status = persistent_volume
                .status
                .get_or_insert_with(Default::default);
            persistent_volume_changed |=
                apply_persistent_volume_csi_observation(&mut status.conditions, &stats);
        }
        if persistent_volume_changed {
            pv_api
                .replace(&volume.volume_name, persistent_volume)
                .await?;
        }

        let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(mut claim) = pvc_api.get(&volume.claim_name).await? else {
            return Ok(());
        };
        let mut claim_changed = false;
        {
            let status = claim
                .status
                .get_or_insert_with(PersistentVolumeClaimStatus::default);
            claim_changed |=
                apply_persistent_volume_claim_csi_observation(&mut status.conditions, &stats);
        }
        if claim_changed {
            pvc_api.replace(&volume.claim_name, claim).await?;
        }

        Ok(())
    }

    pub(super) async fn mark_volume_attached(
        &self,
        volume_name: &str,
        attached: bool,
    ) -> Result<(), ReconcileError> {
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(mut volume) = api.get(volume_name).await? else {
            return Ok(());
        };
        let status = volume.status.get_or_insert_with(Default::default);
        status.attached_node = attached.then(|| self.node_name.clone());
        if status.phase.is_none() {
            status.phase = Some("Bound".to_string());
        }
        api.replace(volume_name, volume).await?;
        Ok(())
    }

    async fn mark_volume_node_expanded(
        &self,
        namespace: &str,
        claim_name: &str,
        volume_name: &str,
        capacity_bytes: i64,
    ) -> Result<(), ReconcileError> {
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(mut volume) = api.get(volume_name).await? else {
            return Ok(());
        };
        if let Some(spec) = volume.spec.as_mut() {
            spec.capacity_bytes = Some(capacity_bytes);
        }
        let status = volume.status.get_or_insert_with(Default::default);
        status.phase = Some("Bound".to_string());
        status.capacity_bytes = Some(capacity_bytes);
        status.node_expansion_required = Some(false);
        status.attached_node = Some(self.node_name.clone());
        api.replace(volume_name, volume).await?;
        self.mark_claim_node_expanded(namespace, claim_name, capacity_bytes)
            .await?;
        Ok(())
    }

    async fn mark_claim_node_expanded(
        &self,
        namespace: &str,
        claim_name: &str,
        capacity_bytes: i64,
    ) -> Result<(), ReconcileError> {
        let api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(mut claim) = api.get(claim_name).await? else {
            return Ok(());
        };
        let status = claim
            .status
            .get_or_insert_with(PersistentVolumeClaimStatus::default);
        let mut changed = false;
        if status.phase.as_deref() != Some("Bound") {
            status.phase = Some("Bound".to_string());
            changed = true;
        }
        if status.capacity_bytes != Some(capacity_bytes) {
            status.capacity_bytes = Some(capacity_bytes);
            changed = true;
        }
        if status.resize_pending != Some(false) {
            status.resize_pending = Some(false);
            changed = true;
        }
        if changed {
            api.replace(claim_name, claim).await?;
        }
        Ok(())
    }
}
