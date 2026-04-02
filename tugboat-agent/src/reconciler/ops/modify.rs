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
use crate::reconciler::ops::add::build_runtime_spec_state;
use crate::reconciler::reconcile::AppendStatus;
use crate::runtime::RuntimeSpecState;
use crate::runtime::error::RuntimeError;
use tracing::{debug, error, info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipMigrationStatus, ShipSpec};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::migrate::VmMigrationPhase;

use super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};

#[derive(Debug, PartialEq, Eq)]
struct HotplugPlan {
    cpu_cores: u64,
    memory_size: u64,
}

impl ShipReconciler {
    pub(crate) async fn reconcile_modified(&self, ship: Ship) -> Result<(), ReconcileError> {
        let Some(ship_metadata) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(ship_id) = &ship_metadata.uid else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.uid".to_string(),
            ));
        };
        let namespace = ship_metadata
            .namespace
            .clone()
            .unwrap_or("default".to_string());

        if !self.runtime_operator.has_ship(ship_id).await {
            if let Some(spec) = &ship.spec {
                if spec.target_node_name.as_deref() == Some(self.node_name.as_str()) {
                    if let Some(status) = &ship.status {
                        if let Some(migration) = &status.migration {
                            if migration.phase == PHASE_FAILED {
                                info!(
                                    "Ship '{}' is a failed migration target and not running, ignoring",
                                    ship_id
                                );
                                return Ok(());
                            }
                        }
                    }
                }
            }
            info!(
                "Ship modified but not running, treating as added: {}",
                ship_id
            );
            return self.reconcile_added(ship).await;
        }

        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };

        let fingerprints = super::ShipFingerprints::new(ship_spec)?;

        let spec_changed = !self
            .runtime_operator
            .matches_spec_fingerprint(ship_id, &fingerprints.spec)
            .await;
        let pvc_changed = !self
            .runtime_operator
            .matches_pvc_volume_fingerprint(ship_id, &fingerprints.pvc_volume)
            .await;
        let mat_changed = !self
            .runtime_operator
            .matches_materialized_volume_fingerprint(ship_id, &fingerprints.materialized_volume)
            .await;

        if !spec_changed && !pvc_changed && !mat_changed {
            debug!("Ship '{}' runtime-significant spec is unchanged", ship_id);
            // Even though the spec is unchanged, check for pending volume expansions
            // since PV status updates are external to the Ship resource.
            self.check_pending_volume_expansions(ship_id, &namespace, ship_spec)
                .await;
            return Ok(());
        }

        if spec_changed && self.try_reconcile_migration(&ship, ship_id).await? {
            return Ok(());
        }

        if spec_changed
            && !pvc_changed
            && self
                .try_reconcile_hotplug(&ship, ship_id, &fingerprints)
                .await?
        {
            return Ok(());
        }

        if spec_changed || pvc_changed {
            if spec_changed {
                info!(
                    "Ship '{}' VM spec changed (image/class/network/uefi); recreating",
                    ship_id
                );
            }
            if pvc_changed {
                info!(
                    "Ship '{}' PVC volume references changed; recreating",
                    ship_id
                );
            }
            return self.reconcile_recreate(ship).await;
        }

        // Only materialized volumes (ConfigMap / Secret) changed — refresh in-place.
        debug!(
            "Ship '{}' materialized volumes changed; refreshing in-place",
            ship_id
        );
        self.refresh_materialized_volumes_for_ship_with_fingerprint(
            ship,
            Some(fingerprints.materialized_volume),
        )
        .await
    }

    /// Recreate the VM by deleting it and then adding it again.
    ///
    /// Used when fields that cannot be mutated in-place (image, ship_class,
    /// network_class_ref, uefi, or PVC volume references) have changed.
    ///
    /// If the agent crashes between the delete and add, the next reconcile event
    /// will find no runtime record for this ship and fall back to `reconcile_added`,
    /// so the operation is safe and recoverable.
    async fn reconcile_recreate(&self, ship: Ship) -> Result<(), ReconcileError> {
        self.reconcile_deleted(ship.clone()).await?;
        self.reconcile_added(ship).await
    }

    /// Re-materialize all ConfigMap / Secret volumes for a running ship without
    /// stopping the VM.  The files are written to the host directory that is
    /// already shared into the guest via virtio-9p, so the guest sees the
    /// updated content through the existing mount.
    async fn refresh_materialized_volumes(
        &self,
        ship_id: &str,
        namespace: &str,
        ship_spec: &ShipSpec,
    ) -> Result<(), ReconcileError> {
        let volumes = self.get_related_volumes(namespace, ship_spec).await?;
        for volume in &volumes {
            let Some(volume) = volume.materialized() else {
                continue;
            };
            if let Err(err) = self.materialize_volume(ship_id, volume) {
                warn!(
                    "Failed to refresh materialized volume '{}' for ship '{}': {}",
                    volume.name, ship_id, err
                );
                return Err(err);
            }
        }
        Ok(())
    }

    async fn try_reconcile_hotplug(
        &self,
        ship: &Ship,
        ship_id: &str,
        fingerprints: &super::ShipFingerprints,
    ) -> Result<bool, ReconcileError> {
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };
        if ship_spec.target_node_name.is_some() {
            return Ok(false);
        }
        let Some(current_state) = self.runtime_operator.spec_state(ship_id).await else {
            return Ok(false);
        };
        let Some(class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
            return Err(ReconcileError::ShipClassNotFound(
                ship_spec.ship_class.clone(),
            ));
        };
        let desired_state = build_runtime_spec_state(ship_spec, &class)?;
        let Some(plan) = plan_hotplug(&current_state, &desired_state) else {
            return Ok(false);
        };
        let Some(name) = ship.name() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship.namespace().unwrap_or("default");
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);

        if let Err(err) = self
            .runtime_operator
            .hotplug_resources(
                ship_id,
                fingerprints.spec.clone(),
                plan.cpu_cores,
                plan.memory_size,
            )
            .await
        {
            let mut status_ship = ship.clone();
            status_ship.append_status(ShipCondition {
                status: "VmHotplugFailed".to_string(),
                message: format!("Failed to update VM resources in place: {err}"),
                timestamp: Some(Time::now()),
            });
            let _ = api.replace_status(name, status_ship).await;
            return Err(err.into());
        }

        let mut status_ship = ship.clone();
        status_ship.append_status(ShipCondition {
            status: "VmHotplugged".to_string(),
            message: format!(
                "Updated VM resources in place to {} vCPU(s) and {} bytes memory",
                plan.cpu_cores, plan.memory_size
            ),
            timestamp: Some(Time::now()),
        });
        api.replace_status(name, status_ship).await?;
        Ok(true)
    }

    async fn try_reconcile_migration(
        &self,
        ship: &Ship,
        ship_id: &str,
    ) -> Result<bool, ReconcileError> {
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };
        let Some(target_node_name) = ship_spec.target_node_name.clone() else {
            return Ok(false);
        };

        if target_node_name == self.node_name {
            if let Some(status) = &ship.status {
                if let Some(migration) = &status.migration {
                    if migration.phase == PHASE_FAILED {
                        if self.runtime_operator.has_ship(ship_id).await {
                            info!(
                                "Migration failed for ship '{}', cleaning up incoming VM on target node",
                                ship_id
                            );
                            if let Err(err) = self.reconcile_deleted(ship.clone()).await {
                                error!(
                                    "Failed to clean up incoming VM for failed migration '{}': {}",
                                    ship_id, err
                                );
                            }
                        }
                        return Ok(true);
                    }
                }
            }
            return Ok(self.runtime_operator.has_ship(ship_id).await);
        }

        if ship_spec.node_name.as_deref() != Some(self.node_name.as_str()) {
            return Ok(false);
        }

        let Some(name) = ship.name() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship.namespace().unwrap_or("default");
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let migration_status = ship
            .status
            .as_ref()
            .and_then(|status| status.migration.clone());

        let Some(migration_status) = migration_status else {
            self.update_migration_status(
                &api,
                name,
                ShipMigrationStatus {
                    phase: PHASE_PENDING.to_string(),
                    source_node_name: ship_spec.node_name.clone(),
                    target_node_name: Some(target_node_name),
                    target_address: None,
                    target_port: None,
                    message: "Waiting for target node to prepare migration receiver".to_string(),
                    timestamp: Some(Time::now()),
                },
                "VmMigrationPending",
                "Waiting for target node to prepare migration receiver".to_string(),
            )
            .await?;
            return Ok(true);
        };

        match migration_status.phase.as_str() {
            PHASE_PENDING => {
                // Waiting for the target node to start QEMU in incoming mode.
                return Ok(true);
            }
            PHASE_READY => {
                // Target is ready; issue the non-blocking QMP migrate command.
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };

                if let Err(err) = self
                    .runtime_operator
                    .migrate(ship_id, target_address.clone(), target_port as u16)
                    .await
                {
                    self.update_migration_status(
                        &api,
                        name,
                        ShipMigrationStatus {
                            phase: PHASE_FAILED.to_string(),
                            source_node_name: Some(self.node_name.clone()),
                            target_node_name: Some(target_node_name.clone()),
                            target_address: Some(target_address.clone()),
                            target_port: Some(target_port),
                            message: format!("Failed to start live migration: {err}"),
                            timestamp: Some(Time::now()),
                        },
                        "VmMigrationFailed",
                        format!("Failed to start live migration: {err}"),
                    )
                    .await?;
                    return Err(err.into());
                }

                // Migration command dispatched; update the phase so the next
                // reconcile event (triggered by this status change) will poll
                // progress rather than re-issuing the migrate command.
                self.update_migration_status(
                    &api,
                    name,
                    ShipMigrationStatus {
                        phase: PHASE_MIGRATING.to_string(),
                        source_node_name: Some(self.node_name.clone()),
                        target_node_name: Some(target_node_name),
                        target_address: Some(target_address),
                        target_port: Some(target_port),
                        message: "Live migration in progress".to_string(),
                        timestamp: Some(Time::now()),
                    },
                    "VmMigrating",
                    "Live migration in progress".to_string(),
                )
                .await?;
                Ok(true)
            }
            PHASE_MIGRATING => {
                // Poll migration progress once per reconcile event instead of
                // spinning inside the reconcile loop.
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };
                let target_node_name = migration_status
                    .target_node_name
                    .clone()
                    .unwrap_or(target_node_name);

                let phase = match self.runtime_operator.check_migration_status(ship_id).await {
                    Ok(phase) => phase,
                    Err(err) => {
                        warn!(
                            "Failed to check migration status for ship '{}': {}",
                            ship_id, err
                        );
                        return Ok(true); // retry on next event
                    }
                };

                match phase {
                    VmMigrationPhase::Completed => {
                        let patch = serde_json::json!({
                            "spec": {
                                "nodeName": target_node_name,
                                "targetNodeName": null,
                            },
                            "status": {
                                "migration": {
                                    "phase": PHASE_COMPLETED,
                                    "sourceNodeName": self.node_name,
                                    "targetNodeName": target_node_name,
                                    "targetAddress": target_address,
                                    "targetPort": target_port,
                                    "message": "Live migration completed successfully",
                                    "timestamp": Time::now(),
                                },
                                "conditions": [
                                    {
                                        "status": "VmMigrated",
                                        "message": format!("VM migrated successfully to node '{target_node_name}'"),
                                        "timestamp": Time::now(),
                                    }
                                ]
                            }
                        });
                        api.patch(name, patch).await?;

                        if let Err(err) =
                            self.runtime_operator.finish_source_migration(ship_id).await
                        {
                            error!(
                                "Failed to clean up migrated source VM for ship '{}': {}",
                                ship_id, err
                            );
                            // API server is already updated; log and continue.
                        }
                        Ok(true)
                    }
                    VmMigrationPhase::Failed | VmMigrationPhase::Cancelled => {
                        let message = format!("Live migration did not complete (phase: {phase:?})");
                        self.update_migration_status(
                            &api,
                            name,
                            ShipMigrationStatus {
                                phase: PHASE_FAILED.to_string(),
                                source_node_name: Some(self.node_name.clone()),
                                target_node_name: Some(target_node_name),
                                target_address: Some(target_address),
                                target_port: Some(target_port),
                                message: message.clone(),
                                timestamp: Some(Time::now()),
                            },
                            "VmMigrationFailed",
                            message.clone(),
                        )
                        .await?;
                        Err(ReconcileError::Runtime(RuntimeError::MigrationFailed(
                            message,
                        )))
                    }
                    _ => {
                        // Still active (Setup, Active, None); wait for next event.
                        Ok(true)
                    }
                }
            }
            // PHASE_COMPLETED, PHASE_FAILED, or any unknown terminal phase.
            _ => Ok(true),
        }
    }

    pub(crate) async fn refresh_materialized_volumes_for_ship(
        &self,
        ship: Ship,
    ) -> Result<(), ReconcileError> {
        self.refresh_materialized_volumes_for_ship_with_fingerprint(ship, None)
            .await
    }

    async fn refresh_materialized_volumes_for_ship_with_fingerprint(
        &self,
        ship: Ship,
        materialized_volume_fingerprint: Option<String>,
    ) -> Result<(), ReconcileError> {
        let Some(ship_metadata) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(ship_id) = &ship_metadata.uid else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.uid".to_string(),
            ));
        };
        let Some(name) = &ship_metadata.name else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship_metadata
            .namespace
            .clone()
            .unwrap_or("default".to_string());
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };

        if !self.runtime_operator.has_ship(ship_id).await {
            debug!(
                "Skipping materialized volume refresh for Ship '{}' because it is not running",
                ship_id
            );
            return Ok(());
        }

        if let Err(err) = self
            .refresh_materialized_volumes(ship_id, &namespace, ship_spec)
            .await
        {
            let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
            let mut status_ship = ship.clone();
            status_ship.append_status(ShipCondition {
                status: "MaterializedVolumesRefreshFailed".to_string(),
                message: format!("Failed to refresh materialized volumes: {}", err),
                timestamp: Some(Time::now()),
            });
            let _ = api.replace_status(name, status_ship).await;
            return Err(err);
        }

        if let Some(fingerprint) = materialized_volume_fingerprint {
            self.runtime_operator
                .update_materialized_volume_fingerprint(ship_id, fingerprint)
                .await;
        }

        let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
        let mut status_ship = ship.clone();
        status_ship.append_status(ShipCondition {
            status: "MaterializedVolumesRefreshed".to_string(),
            message: "Materialized volumes (ConfigMap/Secret) refreshed in-place".to_string(),
            timestamp: Some(Time::now()),
        });
        api.replace_status(name, status_ship).await?;

        self.check_pending_volume_expansions(ship_id, &namespace, ship_spec)
            .await;
        Ok(())
    }

    async fn update_migration_status(
        &self,
        api: &Api<Ship>,
        name: &str,
        migration: ShipMigrationStatus,
        condition_status: &str,
        condition_message: String,
    ) -> Result<(), ReconcileError> {
        let patch = serde_json::json!({
            "status": {
                "migration": migration,
                "conditions": [
                    {
                        "status": condition_status,
                        "message": condition_message,
                        "timestamp": Time::now(),
                    }
                ]
            }
        });
        api.patch_status(name, patch).await?;
        Ok(())
    }

    /// Check whether any attached volumes need CSI node-side expansion and, if so,
    /// perform the expansion and refresh volume stats. This runs even when the Ship
    /// spec itself has not changed, because PV status updates (e.g.
    /// `node_expansion_required`) are external to the Ship resource.
    async fn check_pending_volume_expansions(
        &self,
        ship_id: &str,
        namespace: &str,
        ship_spec: &ShipSpec,
    ) {
        let volumes = match self.get_related_volumes(namespace, ship_spec).await {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "Failed to resolve volumes for expansion check on ship '{}': {}",
                    ship_id, err
                );
                return;
            }
        };
        let published_volumes = match self.csi.load_published_volumes(ship_id) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "Failed to load published volume state for expansion check on ship '{}': {}",
                    ship_id, err
                );
                return;
            }
        };
        for volume in &volumes {
            let Some(volume) = volume.persistent_volume_claim() else {
                continue;
            };
            let Some(published) = published_volumes
                .iter()
                .find(|p| p.claim_name == volume.name)
            else {
                continue;
            };
            let secrets = match self.resolve_csi_secrets(volume).await {
                Ok(s) => s,
                Err(err) => {
                    warn!(
                        "Failed to resolve CSI secrets for volume '{}' expansion check: {}",
                        volume.name, err
                    );
                    continue;
                }
            };
            if let Err(err) = self
                .ensure_node_expansion(namespace, volume, published, &secrets)
                .await
            {
                warn!(
                    "Failed to expand volume '{}' for ship '{}': {}",
                    volume.name, ship_id, err
                );
            }
            if let Err(err) = self
                .refresh_volume_stats(namespace, volume, published)
                .await
            {
                warn!(
                    "Failed to refresh volume stats for '{}' on ship '{}': {}",
                    volume.name, ship_id, err
                );
            }
        }
    }
}

fn plan_hotplug(current: &RuntimeSpecState, desired: &RuntimeSpecState) -> Option<HotplugPlan> {
    if current.image != desired.image
        || current.network_class_ref != desired.network_class_ref
        || current.uefi != desired.uefi
        || desired.cpu_cores < current.cpu_cores
        || desired.memory_size < current.memory_size
    {
        return None;
    }

    Some(HotplugPlan {
        cpu_cores: desired.cpu_cores,
        memory_size: desired.memory_size,
    })
}

#[cfg(test)]
mod tests {
    use super::plan_hotplug;
    use crate::runtime::RuntimeSpecState;

    fn runtime_spec_state(cpu_cores: u64, memory_size: u64) -> RuntimeSpecState {
        RuntimeSpecState {
            image: "registry.example.com/vm:v1".to_string(),
            network_class_ref: Vec::new(),
            uefi: None,
            cpu_cores,
            memory_size,
        }
    }

    #[test]
    fn plans_hotplug_when_only_resources_increase() {
        let current = runtime_spec_state(2, 2 * 1024 * 1024 * 1024);
        let desired = runtime_spec_state(4, 4 * 1024 * 1024 * 1024);

        let plan = plan_hotplug(&current, &desired).expect("hotplug should be allowed");
        assert_eq!(plan.cpu_cores, 4);
        assert_eq!(plan.memory_size, 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn rejects_hotplug_when_resources_decrease() {
        let current = runtime_spec_state(4, 4 * 1024 * 1024 * 1024);
        let desired = runtime_spec_state(2, 2 * 1024 * 1024 * 1024);

        assert!(plan_hotplug(&current, &desired).is_none());
    }

    #[test]
    fn rejects_hotplug_when_non_resource_fields_change() {
        let current = runtime_spec_state(2, 2 * 1024 * 1024 * 1024);
        let mut desired = runtime_spec_state(4, 4 * 1024 * 1024 * 1024);
        desired.image = "registry.example.com/vm:v2".to_string();

        assert!(plan_hotplug(&current, &desired).is_none());
    }
}
