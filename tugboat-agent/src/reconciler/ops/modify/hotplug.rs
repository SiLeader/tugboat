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
use crate::reconciler::volume::VolumeInfo;
use tracing::{debug, warn};
use tugboat_client::Api;
use tugboat_csi_operator::CsiAccessType;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    RuntimeClass, Ship, ShipActualAllocation, ShipClass, ShipCondition, ShipSpec,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

use super::super::ShipFingerprints;
use super::{
    current_nic_ids, current_volume_ids, network_class_info_key, network_ref_key,
    parse_memory_size, pvc_aliases, ship_class_architecture, ship_class_cpu_cores,
    ship_class_memory_bytes, ship_class_memory_size_string,
};

pub(super) struct HotplugPreparedState {
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

impl ShipReconciler {
    pub(super) async fn try_reconcile_hotplug(
        &self,
        ship: &Ship,
        ship_id: &str,
        namespace: &str,
        new_spec: &ShipSpec,
        fingerprints: &ShipFingerprints,
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
