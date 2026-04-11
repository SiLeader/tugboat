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

use crate::csi::{PublishedVolume, access_type_from_volume_mode};
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::ops::hotplug::{HotplugBaseline, HotplugDesired, classify_hotplug_changes};
use crate::reconciler::reconcile::AppendStatus;
use crate::reconciler::volume::{NormalizedVolumeSource, VolumeInfo, normalized_ship_volumes};
use sha2::Digest;
use tracing::{debug, info, warn};
use tugboat_client::Api;
use tugboat_csi_operator::CsiAccessType;
use tugboat_resources::manifests::core::v1::{
    Node, RuntimeClass, Ship, ShipActualAllocation, ShipClass, ShipCondition, ShipSpec,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_resources::sized::SizedString;
use tugboat_resources::{NODE_RUNTIME_CLASS_LABEL_KEY, ObjectMetaResource, ShipMigrationExt};
use tugboat_vm_runtime_interface::hotplug::sanitize_identifier;
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

use super::migration::MigrationStateMachine;
use super::{PHASE_COMPLETED, PHASE_FAILED};

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
            if let Some(spec) = &ship.spec
                && spec.target_node_name.is_some()
                && let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && matches!(
                    migration.phase.as_str(),
                    super::PHASE_PENDING
                        | super::PHASE_READY
                        | super::PHASE_MIGRATING
                        | PHASE_COMPLETED
                )
            {
                let migration_sm = MigrationStateMachine::new(self);
                if migration_sm.try_reconcile(&ship, ship_id).await? {
                    return Ok(());
                }
            }
            if let Some(spec) = &ship.spec
                && spec.node_name.as_deref() == Some(self.node_name.as_str())
                && spec.target_node_name.is_some()
                && let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && migration.phase == PHASE_COMPLETED
            {
                info!(
                    "Ship '{}' already completed source cleanup, finalizing migration cutover",
                    ship_id
                );
                let migration_sm = MigrationStateMachine::new(self);
                if migration_sm.try_reconcile(&ship, ship_id).await? {
                    return Ok(());
                }
            }
            if let Some(spec) = &ship.spec
                && spec.target_node_name.as_deref() == Some(self.node_name.as_str())
                && let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && migration.phase == PHASE_FAILED
            {
                info!(
                    "Ship '{}' is a failed migration target and not running, ignoring",
                    ship_id
                );
                return Ok(());
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

        if spec_changed {
            let migration_sm = MigrationStateMachine::new(self);
            if migration_sm.try_reconcile(&ship, ship_id).await? {
                return Ok(());
            }
        }

        if spec_changed || pvc_changed {
            if ship.has_active_migration() {
                debug!(
                    "Ship '{}' is migrating; skipping hotplug until migration settles",
                    ship_id
                );
                return Ok(());
            }

            if self
                .try_reconcile_hotplug(&ship, ship_id, &namespace, ship_spec, &fingerprints)
                .await?
            {
                return Ok(());
            }

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

    async fn try_reconcile_hotplug(
        &self,
        ship: &Ship,
        ship_id: &str,
        namespace: &str,
        new_spec: &ShipSpec,
        fingerprints: &super::ShipFingerprints,
    ) -> Result<bool, ReconcileError> {
        // Acquire the per-ship hotplug lock before reading baseline state.
        // This serializes concurrent reconcile loops for the same ship and prevents
        // a second loop from applying a stale diff or overwriting state written by
        // the first.
        let Some(hotplug_lock) = self.runtime_operator.get_hotplug_lock(ship_id).await else {
            return Ok(false);
        };
        let _hotplug_guard = hotplug_lock.lock().await;

        // Re-validate fingerprints after acquiring the lock: a concurrent hotplug
        // may have already applied the desired changes while we were waiting.
        let spec_now_matches = self
            .runtime_operator
            .matches_spec_fingerprint(ship_id, &fingerprints.spec)
            .await;
        let pvc_now_matches = self
            .runtime_operator
            .matches_pvc_volume_fingerprint(ship_id, &fingerprints.pvc_volume)
            .await;
        if spec_now_matches && pvc_now_matches {
            debug!(
                "Ship '{}' fingerprints already match after acquiring hotplug lock; skipping",
                ship_id
            );
            return Ok(true);
        }

        let Some(prepared) = self
            .prepare_hotplug_state(ship, ship_id, namespace, new_spec)
            .await?
        else {
            return Ok(false);
        };

        let baseline = HotplugBaseline {
            current_cpu_cores: ship
                .status
                .as_ref()
                .and_then(|status| status.actual_allocation.as_ref())
                .and_then(|alloc| alloc.cpu_cores)
                .unwrap_or(ship_class_cpu_cores(&prepared.old_class)?),
            current_memory_bytes: ship
                .status
                .as_ref()
                .and_then(|status| status.actual_allocation.as_ref())
                .and_then(|alloc| alloc.memory_size.as_deref())
                .and_then(parse_memory_size)
                .unwrap_or(ship_class_memory_bytes(&prepared.old_class)?),
            current_nic_ids: current_nic_ids(
                ship,
                ship_id,
                &prepared.old_spec,
                &prepared.old_network_classes,
            ),
            current_volume_ids: current_volume_ids(
                ship,
                &prepared.old_spec,
                &prepared.current_published_volumes,
            )?,
        };

        let plan = classify_hotplug_changes(
            ship_id,
            &prepared.old_spec,
            new_spec,
            &baseline,
            &HotplugDesired {
                desired_cpu_cores: ship_class_cpu_cores(&prepared.new_class)?,
                desired_memory_bytes: ship_class_memory_bytes(&prepared.new_class)?,
                memory_size: ship_class_memory_size_string(&prepared.new_class)?,
                nics_added: prepared
                    .added_network_plans
                    .iter()
                    .map(|plan| plan.vm.clone())
                    .collect(),
                volumes_added: prepared.added_vm_volumes.clone(),
            },
            prepared
                .runtime_class
                .spec
                .as_ref()
                .and_then(|spec| spec.hotplug.as_ref()),
            prepared.old_spec.image != new_spec.image
                || prepared.old_spec.uefi != new_spec.uefi
                || prepared.old_spec.target_node_name != new_spec.target_node_name
                || ship_class_architecture(&prepared.old_class)?
                    != ship_class_architecture(&prepared.new_class)?,
        );

        if plan.has_unsupported_changes {
            best_effort_cleanup_hotplug_additions(
                self,
                ship_id,
                namespace,
                &prepared.added_network_plans,
                &prepared.added_published_volumes,
                &prepared.new_volumes,
            )
            .await;
            return Ok(false);
        }

        let Some(hotplug_req) = plan.hotplug_req.clone() else {
            best_effort_cleanup_hotplug_additions(
                self,
                ship_id,
                namespace,
                &prepared.added_network_plans,
                &prepared.added_published_volumes,
                &prepared.new_volumes,
            )
            .await;
            return Ok(false);
        };

        for plan in &prepared.added_network_plans {
            if let Err(err) = self.cni.add_single(ship_id, plan.clone()).await {
                self.handle_hotplug_recreate_failure(
                    ship,
                    ship_id,
                    namespace,
                    &prepared,
                    format!("Failed to prepare hotplug network: {err}"),
                    "Hotplug preparation failed",
                )
                .await?;
                return Ok(true);
            }
        }

        if let Err(err) = self.runtime_operator.hotplug(hotplug_req).await {
            self.handle_hotplug_recreate_failure(
                ship,
                ship_id,
                namespace,
                &prepared,
                format!("Failed to hotplug VM resources: {err}"),
                "Hotplug failed",
            )
            .await?;
            return Ok(true);
        }

        let old_network_plans = self
            .cni
            .create_network_configs(ship_id, prepared.old_network_classes.clone());
        let removed_network_plans = old_network_plans
            .into_iter()
            .filter(|plan| {
                prepared
                    .removed_network_keys
                    .contains(&network_class_info_key(&plan.info))
            })
            .collect::<Vec<_>>();
        for plan in removed_network_plans {
            let iface_name = plan.vm.iface_name.clone();
            let network_key = network_class_info_key(&plan.info);
            if let Err(err) = self.cni.del_single(ship_id, plan).await {
                self.handle_hotplug_recreate_failure(
                    ship,
                    ship_id,
                    namespace,
                    &prepared,
                    format!(
                        "Failed to clean up removed hotplug network '{network_key}' ({iface_name}): {err}"
                    ),
                    "Hotplug cleanup failed",
                )
                .await?;
                return Ok(true);
            }
        }

        let removed_published_volumes = prepared
            .current_published_volumes
            .iter()
            .filter(|volume| prepared.removed_volume_aliases.contains(&volume.claim_name))
            .cloned()
            .collect::<Vec<_>>();
        if !removed_published_volumes.is_empty() {
            let mut secrets =
                std::collections::HashMap::with_capacity(removed_published_volumes.len());
            for published in &removed_published_volumes {
                let Some(volume) = prepared.old_volumes.iter().find_map(|volume| {
                    let volume = volume.persistent_volume_claim()?;
                    (volume.name == published.claim_name).then_some(volume)
                }) else {
                    return Err(ReconcileError::PersistentVolumeClaimNotFound(
                        published.claim_name.clone(),
                    ));
                };
                let resolved = self.resolve_csi_secrets(volume).await?;
                secrets.insert(published.claim_name.clone(), resolved.controller_publish);
            }

            self.cleanup_published_volumes(&removed_published_volumes, &secrets)
                .await?;

            for volume in &prepared.old_volumes {
                let Some(volume) = volume.persistent_volume_claim() else {
                    continue;
                };
                if prepared.removed_volume_aliases.contains(&volume.name)
                    && let Err(err) = self.mark_volume_attached(&volume.volume_name, false).await
                {
                    warn!(
                        "Failed to clear attachment for hot-unplugged volume '{}' on ship '{}': {}",
                        volume.volume_name, ship_id, err
                    );
                }
            }
        }

        let mut next_published_volumes = prepared
            .current_published_volumes
            .into_iter()
            .filter(|volume| !prepared.removed_volume_aliases.contains(&volume.claim_name))
            .collect::<Vec<_>>();
        next_published_volumes.extend(prepared.added_published_volumes);
        self.runtime_operator
            .update_runtime_state(
                ship_id,
                new_spec.clone(),
                fingerprints.clone(),
                next_published_volumes,
            )
            .await;

        self.patch_hotplug_success_status(
            namespace,
            &prepared.ship_name,
            ship,
            plan.actual_allocation,
        )
        .await?;

        self.check_pending_volume_expansions(ship_id, namespace, new_spec)
            .await;
        Ok(true)
    }

    async fn prepare_hotplug_state(
        &self,
        ship: &Ship,
        ship_id: &str,
        namespace: &str,
        new_spec: &ShipSpec,
    ) -> Result<Option<HotplugPreparedState>, ReconcileError> {
        let Some(old_spec) = self.runtime_operator.current_ship_spec(ship_id).await else {
            return Ok(None);
        };

        let Some(ship_meta) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(ship_name) = ship_meta.name.clone() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };

        let Some(old_class) = self.ship_class_api.get(&old_spec.ship_class).await? else {
            return Ok(None);
        };
        let Some(new_class) = self.ship_class_api.get(&new_spec.ship_class).await? else {
            return Ok(None);
        };
        let Some(runtime_class) = self.resolve_local_runtime_class(new_spec).await? else {
            return Ok(None);
        };

        let old_network_classes = self
            .get_related_network_classes(namespace, &old_spec)
            .await?;
        let (added_network_plans, removed_network_keys) = self
            .diff_hotplug_networks(ship_id, &old_spec, new_spec, namespace)
            .await?;

        let old_volumes = self.get_related_volumes(namespace, &old_spec).await?;
        let new_volumes = self.get_related_volumes(namespace, new_spec).await?;
        let Some((added_published_volumes, added_vm_volumes, removed_volume_aliases)) = self
            .prepare_hotplug_volume_changes(ship_id, namespace, &old_spec, new_spec, &new_volumes)
            .await?
        else {
            return Ok(None);
        };
        let current_published_volumes = self
            .runtime_operator
            .current_published_volumes(ship_id)
            .await
            .unwrap_or_default();

        Ok(Some(HotplugPreparedState {
            ship_name,
            old_spec,
            old_class,
            new_class,
            runtime_class,
            old_network_classes,
            old_volumes,
            new_volumes,
            added_network_plans,
            removed_network_keys,
            current_published_volumes,
            added_published_volumes,
            added_vm_volumes,
            removed_volume_aliases,
        }))
    }

    async fn diff_hotplug_networks(
        &self,
        ship_id: &str,
        old_spec: &ShipSpec,
        new_spec: &ShipSpec,
        namespace: &str,
    ) -> Result<(Vec<crate::cni::PlannedNetworkConfig>, Vec<String>), ReconcileError> {
        let old_network_keys = old_spec
            .network_class_ref
            .iter()
            .map(network_ref_key)
            .collect::<Vec<_>>();
        let new_network_keys = new_spec
            .network_class_ref
            .iter()
            .map(network_ref_key)
            .collect::<Vec<_>>();
        let added_network_keys = new_network_keys
            .iter()
            .filter(|key| !old_network_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        let removed_network_keys = old_network_keys
            .iter()
            .filter(|key| !new_network_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();

        let new_network_classes = self
            .get_related_network_classes(namespace, new_spec)
            .await?;
        let added_network_classes = new_network_classes
            .into_iter()
            .filter(|info| added_network_keys.contains(&network_class_info_key(info)))
            .collect::<Vec<_>>();
        let added_network_plans = self.cni.create_network_configs_from_index(
            ship_id,
            old_spec.network_class_ref.len(),
            added_network_classes,
        );

        Ok((added_network_plans, removed_network_keys))
    }

    async fn prepare_hotplug_volume_changes(
        &self,
        ship_id: &str,
        namespace: &str,
        old_spec: &ShipSpec,
        new_spec: &ShipSpec,
        new_volumes: &[VolumeInfo],
    ) -> Result<Option<(Vec<PublishedVolume>, Vec<VmVolumeConfig>, Vec<String>)>, ReconcileError>
    {
        let old_pvc_aliases = pvc_aliases(old_spec)?;
        let new_pvc_aliases = pvc_aliases(new_spec)?;
        let added_volume_aliases = new_pvc_aliases
            .iter()
            .filter(|alias| !old_pvc_aliases.contains(*alias))
            .cloned()
            .collect::<Vec<_>>();
        let removed_volume_aliases = old_pvc_aliases
            .iter()
            .filter(|alias| !new_pvc_aliases.contains(*alias))
            .cloned()
            .collect::<Vec<_>>();

        let added_volume_infos = new_volumes
            .iter()
            .filter(|volume| {
                volume
                    .persistent_volume_claim()
                    .map(|volume| added_volume_aliases.contains(&volume.name))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        if added_volume_infos.iter().any(|volume| {
            volume.persistent_volume_claim().is_some_and(|volume| {
                access_type_from_volume_mode(volume.volume.volume_mode.as_deref())
                    .map(|access_type| access_type != CsiAccessType::Block)
                    .unwrap_or(true)
            })
        }) {
            return Ok(None);
        }

        let (added_published_volumes, added_vm_volumes) = if added_volume_infos.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            self.setup_volumes(ship_id, namespace, &added_volume_infos)
                .await?
        };

        Ok(Some((
            added_published_volumes,
            added_vm_volumes,
            removed_volume_aliases,
        )))
    }

    async fn handle_hotplug_recreate_failure(
        &self,
        ship: &Ship,
        ship_id: &str,
        namespace: &str,
        prepared: &HotplugPreparedState,
        status_message: String,
        log_prefix: &str,
    ) -> Result<(), ReconcileError> {
        best_effort_cleanup_hotplug_additions(
            self,
            ship_id,
            namespace,
            &prepared.added_network_plans,
            &prepared.added_published_volumes,
            &prepared.new_volumes,
        )
        .await;
        self.mark_hotplug_failed(ship, status_message).await?;
        warn!("{log_prefix} for ship '{}': recreating", ship_id);
        self.reconcile_recreate(ship.clone()).await
    }

    async fn patch_hotplug_success_status(
        &self,
        namespace: &str,
        ship_name: &str,
        ship: &Ship,
        actual_allocation: ShipActualAllocation,
    ) -> Result<(), ReconcileError> {
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let mut status = ship.status.clone().unwrap_or_default();
        status.actual_allocation = Some(actual_allocation);
        status.append_status(ShipCondition {
            status: "Hotplugged".to_string(),
            message: "Applied supported CPU/memory/network/storage changes in-place".to_string(),
            timestamp: Some(Time::now()),
        });
        api.patch_status(
            ship_name,
            serde_json::json!({
                "status": {
                    "actualAllocation": status.actual_allocation,
                    "conditions": status.conditions,
                }
            }),
        )
        .await?;
        Ok(())
    }

    async fn mark_hotplug_failed(
        &self,
        ship: &Ship,
        message: String,
    ) -> Result<(), ReconcileError> {
        let Some(meta) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(name) = meta.name.as_deref() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = meta
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
        let mut status_ship = ship.clone();
        status_ship.append_status(ShipCondition {
            status: "HotplugFailed".to_string(),
            message,
            timestamp: Some(Time::now()),
        });
        api.replace_status(name, status_ship).await?;
        Ok(())
    }

    async fn resolve_local_runtime_class(
        &self,
        ship_spec: &ShipSpec,
    ) -> Result<Option<RuntimeClass>, ReconcileError> {
        let node_name = ship_spec
            .node_name
            .as_deref()
            .unwrap_or(self.node_name.as_str());
        let node_api: Api<Node> = Api::all(self.client.clone());
        let Some(node) = node_api.get(node_name).await? else {
            return Ok(None);
        };
        let runtime_class_name = node
            .object_meta
            .as_ref()
            .and_then(|meta| meta.labels.get(NODE_RUNTIME_CLASS_LABEL_KEY))
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(runtime_class_name) = runtime_class_name else {
            return Ok(None);
        };
        let runtime_class_api: Api<RuntimeClass> = Api::all(self.client.clone());
        runtime_class_api
            .get(runtime_class_name)
            .await
            .map_err(Into::into)
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
        let published_volumes = match self.csi.load_published_volumes(ship_id).await {
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

struct HotplugPreparedState {
    ship_name: String,
    old_spec: ShipSpec,
    old_class: ShipClass,
    new_class: ShipClass,
    runtime_class: RuntimeClass,
    old_network_classes: Vec<crate::cni::NetworkClassInfo>,
    old_volumes: Vec<VolumeInfo>,
    new_volumes: Vec<VolumeInfo>,
    added_network_plans: Vec<crate::cni::PlannedNetworkConfig>,
    removed_network_keys: Vec<String>,
    current_published_volumes: Vec<PublishedVolume>,
    added_published_volumes: Vec<PublishedVolume>,
    added_vm_volumes: Vec<VmVolumeConfig>,
    removed_volume_aliases: Vec<String>,
}

fn network_ref_key(
    reference: &tugboat_resources::manifests::core::v1::ShipNetworkClassReference,
) -> String {
    format!(
        "{}|{}|{}",
        reference.api_group, reference.kind, reference.name
    )
}

fn network_class_info_key(info: &crate::cni::NetworkClassInfo) -> String {
    let kind = if info.namespace.is_some() {
        "NetworkClass"
    } else {
        "ClusterNetworkClass"
    };
    format!("core|{kind}|{}", info.name)
}

fn ship_class_spec(
    ship_class: &ShipClass,
) -> Result<&tugboat_resources::manifests::core::v1::ShipClassSpec, ReconcileError> {
    ship_class
        .spec
        .as_ref()
        .ok_or_else(|| ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec".to_string()))
}

fn ship_class_cpu_cores(ship_class: &ShipClass) -> Result<u64, ReconcileError> {
    ship_class_spec(ship_class)?
        .cpu
        .as_ref()
        .map(|cpu| cpu.cores)
        .ok_or_else(|| {
            ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec.cpu".to_string())
        })
}

fn ship_class_architecture(ship_class: &ShipClass) -> Result<&str, ReconcileError> {
    ship_class_spec(ship_class)?
        .cpu
        .as_ref()
        .map(|cpu| cpu.architecture.as_str())
        .ok_or_else(|| {
            ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec.cpu".to_string())
        })
}

fn ship_class_memory_size_string(ship_class: &ShipClass) -> Result<String, ReconcileError> {
    ship_class_spec(ship_class)?
        .memory
        .as_ref()
        .map(|memory| memory.size.clone())
        .ok_or_else(|| {
            ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec.memory".to_string())
        })
}

fn ship_class_memory_bytes(ship_class: &ShipClass) -> Result<u64, ReconcileError> {
    let size = ship_class_memory_size_string(ship_class)?;
    parse_memory_size(&size).ok_or_else(|| {
        ReconcileError::Runtime(crate::runtime::error::RuntimeError::MemorySize(size))
    })
}

fn parse_memory_size(value: &str) -> Option<u64> {
    SizedString(value.to_string()).as_byte_length()
}

fn current_nic_ids(
    ship: &Ship,
    ship_id: &str,
    old_spec: &ShipSpec,
    old_network_classes: &[crate::cni::NetworkClassInfo],
) -> Vec<String> {
    let status_ids = ship
        .status
        .as_ref()
        .and_then(|status| status.actual_allocation.as_ref())
        .map(|alloc| alloc.nic_ids.clone())
        .unwrap_or_default();
    if status_ids.len() == old_spec.network_class_ref.len() {
        return status_ids;
    }

    old_network_classes
        .iter()
        .map(|info| {
            let ident = match &info.namespace {
                Some(ns) => format!("NetworkClass/{ns}/{}/{ship_id}", info.name),
                None => format!("ClusterNetworkClass/{}/{ship_id}", info.name),
            };
            let digest = sha2::Sha256::digest(ident.as_bytes());
            let mac = format!("52:54:00:{:02x}:{:02x}:{:02x}", digest[0], digest[1], digest[2]);
            format!("nic-{}", sanitize_identifier(&mac))
        })
        .collect()
}

fn current_volume_ids(
    ship: &Ship,
    old_spec: &ShipSpec,
    published_volumes: &[PublishedVolume],
) -> Result<Vec<String>, ReconcileError> {
    let status_ids = ship
        .status
        .as_ref()
        .and_then(|status| status.actual_allocation.as_ref())
        .map(|alloc| alloc.volume_ids.clone())
        .unwrap_or_default();
    let old_aliases = pvc_aliases(old_spec)?;
    if status_ids.len() == old_aliases.len() {
        return Ok(status_ids);
    }

    Ok(old_aliases
        .into_iter()
        .filter_map(|alias| {
            published_volumes
                .iter()
                .find(|volume| volume.claim_name == alias)
                .map(|volume| format!("dev-{}", sanitize_identifier(&volume.target_path)))
        })
        .collect())
}

fn pvc_aliases(spec: &ShipSpec) -> Result<Vec<String>, ReconcileError> {
    Ok(normalized_ship_volumes(spec)?
        .into_iter()
        .filter_map(|volume| match volume.source {
            NormalizedVolumeSource::PersistentVolumeClaim { .. } => Some(volume.name),
            _ => None,
        })
        .collect())
}

async fn best_effort_cleanup_hotplug_additions(
    reconciler: &ShipReconciler,
    ship_id: &str,
    namespace: &str,
    added_network_plans: &[crate::cni::PlannedNetworkConfig],
    added_published_volumes: &[PublishedVolume],
    volumes: &[VolumeInfo],
) {
    for plan in added_network_plans.iter().rev() {
        if let Err(err) = reconciler.cni.del_single(ship_id, plan.clone()).await {
            warn!(
                "Failed to clean up hotplug network preparation for ship '{}': {}",
                ship_id, err
            );
        }
    }
    if !added_published_volumes.is_empty() {
        match reconciler
            .controller_publish_secret_map(namespace, volumes, added_published_volumes)
            .await
        {
            Ok(secrets) => {
                if let Err(err) = reconciler
                    .cleanup_published_volumes_best_effort(added_published_volumes, &secrets)
                    .await
                {
                    warn!(
                        "Failed to clean up hotplug volume preparation for ship '{}': {}",
                        ship_id, err
                    );
                }
            }
            Err(err) => {
                warn!(
                    "Failed to resolve hotplug cleanup secrets for ship '{}': {}",
                    ship_id, err
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::current_nic_ids;
    use crate::cni::NetworkClassInfo;
    use sha2::Digest;
    use tugboat_resources::manifests::core::v1::{
        NetworkClassSpec, Ship, ShipActualAllocation, ShipNetworkClassReference, ShipSpec,
        ShipStatus,
    };
    use tugboat_vm_runtime_interface::hotplug::sanitize_identifier;

    fn ship_spec_with_networks() -> ShipSpec {
        ShipSpec {
            image: "registry.example.com/test:1".to_string(),
            ship_class: "small".to_string(),
            node_name: Some("node-a".to_string()),
            network_class_ref: vec![ShipNetworkClassReference {
                api_group: "core".to_string(),
                kind: "NetworkClass".to_string(),
                name: "frontend".to_string(),
            }],
            uefi: None,
            tolerations: vec![],
            scheduler_name: None,
            volume_claim_ref: vec![],
            volumes: vec![],
            target_node_name: None,
            runtime_class: None,
        }
    }

    #[test]
    fn current_nic_ids_returns_status_ids_when_lengths_match() {
        let old_spec = ship_spec_with_networks();
        let ship = Ship {
            status: Some(ShipStatus {
                actual_allocation: Some(ShipActualAllocation {
                    nic_ids: vec!["nic-preexisting".to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ids = current_nic_ids(&ship, "ship-123", &old_spec, &[]);
        assert_eq!(ids, vec!["nic-preexisting".to_string()]);
    }

    #[test]
    fn current_nic_ids_fallback_matches_cni_mac_derivation() {
        let old_spec = ship_spec_with_networks();
        let ship = Ship::default();
        let old_network_classes = vec![NetworkClassInfo {
            name: "frontend".to_string(),
            namespace: Some("default".to_string()),
            spec: NetworkClassSpec::default(),
        }];

        let ids = current_nic_ids(&ship, "ship-123", &old_spec, &old_network_classes);
        assert_eq!(ids.len(), 1);

        let ident = "NetworkClass/default/frontend/ship-123";
        let digest = sha2::Sha256::digest(ident.as_bytes());
        let mac = format!("52:54:00:{:02x}:{:02x}:{:02x}", digest[0], digest[1], digest[2]);
        let expected = format!("nic-{}", sanitize_identifier(&mac));

        assert_eq!(ids[0], expected);
    }
}
