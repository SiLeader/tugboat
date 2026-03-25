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
use crate::reconciler::reconcile::AppendStatus;
use crate::reconciler::volume::VolumeInfo;
use crate::runtime::RuntimeCreateRequest;
use tracing::{debug, error, info};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition};
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
        let spec_fingerprint = serde_json::to_string(ship_spec)?;

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
                    spec_fingerprint,
                    published_volumes,
                )
                .await;
            info!("Recovered existing VM runtime state for ship '{}'", ship_id);
            return Ok(());
        }

        let stale_published_volumes = self.csi.load_published_volumes(ship_id)?;
        if !stale_published_volumes.is_empty() {
            info!("Cleaning up stale CSI publish state for ship '{}'", ship_id);
            self.cleanup_published_volumes(&stale_published_volumes)
                .await?;
            self.csi.cleanup_mount_namespace(ship_id)?;
        }

        debug!("Getting network classes for ship");
        let network_classes = self
            .get_related_network_classes(&namespace, ship_spec)
            .await?;
        debug!("{} network classes loaded", network_classes.len());
        debug!("Getting volume claims for ship");
        let volumes = self.get_related_volumes(&namespace, ship_spec).await?;
        debug!("{} volumes loaded", volumes.len());

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
        let (published_volumes, vm_volumes) = self.setup_volumes(ship_id, &volumes).await?;

        self.with_cleanup(ship_id, published_volumes.as_slice(), || {
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
                        spec_fingerprint,
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
        volumes: &[VolumeInfo],
    ) -> Result<(Vec<PublishedVolume>, Vec<VmVolumeConfig>), ReconcileError> {
        let mut published_volumes = Vec::new();
        let mut vm_volumes = Vec::new();
        if !volumes.is_empty() {
            self.csi.ensure_mount_namespace(ship_id)?;
        }
        for volume in volumes {
            let (_, read_only) = effective_publish_settings(
                &volume.claim.access_modes,
                &volume.volume.access_modes,
                volume.source.read_only,
            )?;
            let secrets = self.resolve_node_secrets(volume).await?;
            match self
                .csi
                .publish(
                    ship_id,
                    &volume.claim_name,
                    &volume.volume,
                    &volume.claim,
                    &volume.source,
                    &secrets,
                )
                .await
            {
                Ok(published) => {
                    vm_volumes.push(vm_volume_config(volume, &published, read_only));
                    published_volumes.push(published);
                }
                Err(err) => {
                    if let Err(cleanup_err) =
                        self.cleanup_published_volumes(&published_volumes).await
                    {
                        error!(
                            "Failed to roll back published volumes after publish error: {cleanup_err}"
                        );
                    }
                    if let Err(cleanup_err) = self.csi.cleanup_mount_namespace(ship_id) {
                        error!(
                            "Failed to clean up mount namespace after publish error: {cleanup_err}"
                        );
                    }
                    return Err(err.into());
                }
            }
        }
        Ok((published_volumes, vm_volumes))
    }

    async fn plan_desired_published_volumes(
        &self,
        ship_id: &str,
        volumes: &[VolumeInfo],
    ) -> Result<Vec<PublishedVolume>, ReconcileError> {
        let mut planned = Vec::with_capacity(volumes.len());
        for volume in volumes {
            let access_type = PublishedAccessType::from(access_type_from_volume_mode(
                volume.volume.volume_mode.as_deref(),
            )?);
            let requires_staging = self
                .csi
                .driver_requires_staging(&volume.source.driver)
                .await?;
            planned.push(self.csi.plan_published_volume(
                ship_id,
                &volume.claim_name,
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
                    .cleanup_runtime_and_published_volumes(ship_id, published_volumes)
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
    ) -> Result<(), ReconcileError> {
        let mut errors = Vec::new();
        for volume in published_volumes.iter().rev() {
            if let Err(err) = self.csi.unpublish(volume).await {
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
    ) -> Result<(), ReconcileError> {
        let mut runtime_published_volumes =
            self.runtime_operator.delete(ship_id.to_string()).await?;
        if runtime_published_volumes.is_empty() {
            runtime_published_volumes = self.csi.load_published_volumes(ship_id)?;
        }
        if runtime_published_volumes.is_empty() {
            self.cleanup_published_volumes(fallback_published_volumes)
                .await?;
        } else {
            self.cleanup_published_volumes(&runtime_published_volumes)
                .await?;
        }
        self.csi.cleanup_mount_namespace(ship_id)?;
        Ok(())
    }
}

fn vm_volume_config(
    volume: &VolumeInfo,
    published: &PublishedVolume,
    read_only: bool,
) -> VmVolumeConfig {
    match published.access_type {
        PublishedAccessType::Block => {
            VmVolumeConfig::block(published.target_path.clone(), "raw", read_only)
        }
        PublishedAccessType::Filesystem => VmVolumeConfig::filesystem(
            published.target_path.clone(),
            volume.claim_name.clone(),
            read_only,
        ),
    }
}

fn validate_recovered_published_volumes(
    ship_id: &str,
    mut persisted: Vec<PublishedVolume>,
    planned: &[PublishedVolume],
) -> Result<Vec<PublishedVolume>, ReconcileError> {
    if persisted.is_empty() {
        if planned.is_empty() {
            return Ok(persisted);
        }
        return Err(ReconcileError::MissingRecoveredPublishedVolumeState(
            ship_id.to_string(),
        ));
    }

    persisted.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    let mut planned_sorted = planned.to_vec();
    planned_sorted.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    if persisted != planned_sorted {
        return Err(ReconcileError::RecoveredPublishedVolumeStateMismatch(
            ship_id.to_string(),
        ));
    }

    Ok(persisted)
}

#[cfg(test)]
mod tests {
    use super::validate_recovered_published_volumes;
    use crate::csi::{PublishedAccessType, PublishedVolume};
    use crate::reconciler::error::ReconcileError;

    fn published_volume(target_path: &str) -> PublishedVolume {
        PublishedVolume {
            driver: "example.csi".to_string(),
            volume_id: format!("volume-{target_path}"),
            target_path: target_path.to_string(),
            access_type: PublishedAccessType::Filesystem,
            mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
            staging_target_path: Some(format!("{target_path}.staging")),
        }
    }

    #[test]
    fn recovered_volumes_require_persisted_state_when_volumes_exist() {
        let planned = vec![published_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/data.fs",
        )];

        let err = validate_recovered_published_volumes("ship-uid", Vec::new(), &planned)
            .expect_err("missing persisted state should fail recovery");

        assert!(matches!(
            err,
            ReconcileError::MissingRecoveredPublishedVolumeState(ship)
            if ship == "ship-uid"
        ));
    }

    #[test]
    fn recovered_volumes_reject_state_that_differs_from_plan() {
        let persisted = vec![published_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/data.fs",
        )];
        let planned = vec![published_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/other.fs",
        )];

        let err = validate_recovered_published_volumes("ship-uid", persisted, &planned)
            .expect_err("mismatched persisted state should fail recovery");

        assert!(matches!(
            err,
            ReconcileError::RecoveredPublishedVolumeStateMismatch(ship)
            if ship == "ship-uid"
        ));
    }

    #[test]
    fn recovered_volumes_accept_matching_persisted_state() {
        let persisted = vec![published_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/data.fs",
        )];

        let recovered =
            validate_recovered_published_volumes("ship-uid", persisted.clone(), &persisted)
                .expect("matching persisted state should be accepted");

        assert_eq!(recovered, persisted);
    }
}
