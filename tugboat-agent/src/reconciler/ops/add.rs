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

use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::ops::add_helpers::validate_recovered_published_volumes;
use crate::reconciler::reconcile::AppendStatus;
use crate::runtime::RuntimeCreateRequest;
use tracing::{debug, error, info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition};
use tugboat_resources::manifests::meta::v1::Time;

mod cleanup;
mod migration_target;
mod recovery;
mod restore;
mod volumes;
use recovery::{RecoveredRuntimeAction, recovered_runtime_action, runtime_fingerprints_for_ship};
use restore::{requested_restore_snapshot, resolve_restore_snapshot};

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

        let (restore_handle, restore_source_id) = if let Some(snap_name) = requested_restore_snapshot(&ship) {
            let api: Api<tugboat_resources::manifests::core::v1::ShipSnapshot> =
                Api::namespaced(self.client.clone(), &namespace);
            let snapshot = api.get(snap_name).await?;
            let resolved = resolve_restore_snapshot(snap_name, snapshot.as_ref())?;
            info!(
                "Resolved restore intent for ship '{}' from snapshot '{}' (handle={})",
                ship_id, snap_name, resolved.handle
            );
            {
                let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
                let mut status_ship = ship.clone();
                status_ship.append_status(ShipCondition {
                    status: "SnapshotResolved".to_string(),
                    message: format!("Resolved snapshot '{snap_name}' for restore"),
                    timestamp: Some(Time::now()),
                });
                api.replace_status(name, status_ship).await?;
            }
            (Some(resolved.handle), Some(resolved.source_ship_id))
        } else {
            (None, None)
        };

        let runtime_fingerprints = runtime_fingerprints_for_ship(ship_spec, &self.node_name)?;

        if self.runtime_operator.is_present(ship_id).await? {
            match recovered_runtime_action(&ship, &self.node_name) {
                RecoveredRuntimeAction::CleanupFailedTarget => {
                    info!(
                        "Recovered stale failed migration target for ship '{}', cleaning it up",
                        ship_id
                    );
                    self.reconcile_deleted(ship.clone()).await?;
                    return Ok(());
                }
                RecoveredRuntimeAction::RecreateTarget => {
                    info!(
                        "Recovered incomplete migration target for ship '{}', recreating receiver",
                        ship_id
                    );
                    self.reconcile_deleted(ship.clone()).await?;
                }
                RecoveredRuntimeAction::Register => {}
            }
        }

        if self.runtime_operator.is_present(ship_id).await? {
            let volumes = self
                .get_related_volumes(&namespace, name, ship_id, ship_spec)
                .await?;
            let planned_published_volumes = self
                .plan_desired_published_volumes(ship_id, &volumes)
                .await?;
            let published_volumes = validate_recovered_published_volumes(
                ship_id,
                self.csi.load_published_volumes(ship_id).await?,
                &planned_published_volumes,
            )?;
            self.runtime_operator
                .register_existing(
                    namespace,
                    name.clone(),
                    ship_id.clone(),
                    ship_spec.clone(),
                    runtime_fingerprints,
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
        let volumes = self
            .get_related_volumes(&namespace, name, ship_id, ship_spec)
            .await?;
        debug!("{} volumes loaded", volumes.len());
        let persisted_volumes = self.csi.load_published_volumes(ship_id).await?;

        // Plan desired published volumes for this new ship creation
        let planned_published_volumes = self
            .plan_desired_published_volumes(ship_id, &volumes)
            .await?;
        let recovered_published_volumes = if persisted_volumes.is_empty() {
            match self
                .csi
                .recover_partial_published_volume_state(ship_id, &planned_published_volumes)
                .await?
            {
                Some(recovered_volumes) => {
                    info!(
                        "Recovered CSI published volumes for ship '{}' from partial-publish state",
                        ship_id
                    );
                    Some(recovered_volumes)
                }
                None => None,
            }
        } else {
            match validate_recovered_published_volumes(
                ship_id,
                persisted_volumes.clone(),
                &planned_published_volumes,
            ) {
                Ok(recovered_volumes) => {
                    info!(
                        "Recovered persisted CSI published volumes for ship '{}'",
                        ship_id
                    );
                    Some(recovered_volumes)
                }
                Err(ReconcileError::RecoveredPublishedVolumeStateMismatch(_)) => {
                    warn!(
                        "CSI published volume state mismatch for ship '{}', cleaning up",
                        ship_id
                    );
                    let controller_publish_secrets = self
                        .controller_publish_secret_map(&namespace, &volumes, &persisted_volumes)
                        .await?;
                    self.cleanup_published_volumes_best_effort(
                        &persisted_volumes,
                        &controller_publish_secrets,
                    )
                    .await?;
                    self.csi.cleanup_mount_namespace(ship_id)?;
                    None
                }
                Err(e) => return Err(e),
            }
        };
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
        let (published_volumes, vm_volumes) = match recovered_published_volumes {
            Some(recovered_volumes) => {
                let vm_volumes = self
                    .setup_recovered_published_volumes(
                        ship_id,
                        &namespace,
                        &volumes,
                        &recovered_volumes,
                    )
                    .await?;
                (recovered_volumes, vm_volumes)
            }
            None => self.setup_volumes(ship_id, &namespace, &volumes).await?,
        };
        let incoming_port =
            if ship_spec.target_node_name.as_deref() == Some(self.node_name.as_str()) {
                Some(self.find_available_port().await?)
            } else {
                None
            };
        let ship_for_migration_ready = ship.clone();

        let result = self
            .with_cleanup(ship_id, published_volumes.as_slice(), &volumes, || {
                let volumes = vm_volumes;
                let published_volumes = published_volumes.clone();
                let ship_for_migration_ready = ship_for_migration_ready.clone();
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
                            incoming_port,
                            restore_handle,
                            restore_source_id,
                            networks: networks.iter().map(|n| n.vm.clone()).collect(),
                            volumes,
                            fingerprints: runtime_fingerprints.clone(),
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
                    if let Some(port) = incoming_port {
                        self.mark_migration_target_ready(&ship_for_migration_ready, port)
                            .await?;
                    }
                    Ok(())
                }
            })
            .await;

        if let Err(err) = &result
            && incoming_port.is_some()
            && let Err(status_err) = self
                .mark_migration_target_failed(
                    &ship,
                    format!(
                        "Failed to prepare migration target on node '{}': {err}. Source VM remains authoritative.",
                        self.node_name
                    ),
                )
                .await
        {
            error!("Failed to publish migration target failure status: {status_err}");
        }

        result
    }
}

#[cfg(test)]
mod tests;
