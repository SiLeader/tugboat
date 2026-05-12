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

use crate::csi::PublishedVolume;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::reconcile::AppendStatus;
use crate::reconciler::volume::{NormalizedVolumeSource, normalized_ship_volumes};
use sha2::Digest;
use tracing::{debug, info, warn};
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    Node, RuntimeClass, Ship, ShipClass, ShipCondition, ShipSpec,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_resources::sized::SizedString;
use tugboat_resources::{NODE_RUNTIME_CLASS_LABEL_KEY, ObjectMetaResource, ShipMigrationExt};
use tugboat_vm_runtime_interface::hotplug::sanitize_identifier;

use super::migration::MigrationStateMachine;
use super::{PHASE_COMPLETED, PHASE_FAILED};
mod hotplug;
mod plan;
use plan::{ModifyPlan, plan_modify_action};

struct RuntimeReconfigureExecution<'a> {
    ship: &'a Ship,
    ship_id: &'a str,
    namespace: &'a str,
    ship_spec: &'a ShipSpec,
    fingerprints: &'a super::ShipFingerprints,
    spec_changed: bool,
    pvc_changed: bool,
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
        let ship_name = ship_metadata.name.as_deref().unwrap_or("");
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

        match plan_modify_action(spec_changed, pvc_changed, mat_changed, &fingerprints) {
            ModifyPlan::Unchanged => {
                debug!("Ship '{}' runtime-significant spec is unchanged", ship_id);
                // Even though the spec is unchanged, check for pending volume expansions
                // since PV status updates are external to the Ship resource.
                self.check_pending_volume_expansions(ship_id, &namespace, ship_name, ship_spec)
                    .await;
                Ok(())
            }
            ModifyPlan::RuntimeReconfigure {
                spec_changed,
                pvc_changed,
            } => {
                self.execute_runtime_reconfigure_plan(RuntimeReconfigureExecution {
                    ship: &ship,
                    ship_id,
                    namespace: &namespace,
                    ship_spec,
                    fingerprints: &fingerprints,
                    spec_changed,
                    pvc_changed,
                })
                .await
            }
            ModifyPlan::RefreshMaterializedVolumes {
                materialized_volume_fingerprint,
            } => {
                debug!(
                    "Ship '{}' materialized volumes changed; refreshing in-place",
                    ship_id
                );
                self.refresh_materialized_volumes_for_ship_with_fingerprint(
                    ship.clone(),
                    Some(materialized_volume_fingerprint),
                )
                .await
            }
        }
    }

    async fn execute_runtime_reconfigure_plan(
        &self,
        plan: RuntimeReconfigureExecution<'_>,
    ) -> Result<(), ReconcileError> {
        if plan.spec_changed {
            let migration_sm = MigrationStateMachine::new(self);
            if migration_sm.try_reconcile(plan.ship, plan.ship_id).await? {
                return Ok(());
            }
        }

        if plan.ship.has_active_migration() {
            debug!(
                "Ship '{}' is migrating; skipping hotplug until migration settles",
                plan.ship_id
            );
            return Ok(());
        }

        if self
            .try_reconcile_hotplug(
                plan.ship,
                plan.ship_id,
                plan.namespace,
                plan.ship_spec,
                plan.fingerprints,
            )
            .await?
        {
            return Ok(());
        }

        if plan.spec_changed {
            info!(
                "Ship '{}' VM spec changed (image/class/network/uefi); recreating",
                plan.ship_id
            );
        }
        if plan.pvc_changed {
            info!(
                "Ship '{}' PVC volume references changed; recreating",
                plan.ship_id
            );
        }
        self.reconcile_recreate(plan.ship.clone()).await
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
        ship_name: &str,
        ship_spec: &ShipSpec,
    ) -> Result<(), ReconcileError> {
        let volumes = self
            .get_related_volumes(namespace, ship_name, ship_id, ship_spec)
            .await?;
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
            .refresh_materialized_volumes(ship_id, &namespace, name, ship_spec)
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

        self.check_pending_volume_expansions(ship_id, &namespace, name, ship_spec)
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
        ship_name: &str,
        ship_spec: &ShipSpec,
    ) {
        let volumes = match self
            .get_related_volumes(namespace, ship_name, ship_id, ship_spec)
            .await
        {
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

pub(super) fn network_ref_key(
    reference: &tugboat_resources::manifests::core::v1::ShipNetworkClassReference,
) -> String {
    format!(
        "{}|{}|{}",
        reference.api_group, reference.kind, reference.name
    )
}

pub(super) fn network_class_info_key(info: &crate::cni::NetworkClassInfo) -> String {
    let kind = if info.namespace.is_some() {
        "NetworkClass"
    } else {
        "ClusterNetworkClass"
    };
    format!("core|{kind}|{}", info.name)
}

pub(super) fn ship_class_spec(
    ship_class: &ShipClass,
) -> Result<&tugboat_resources::manifests::core::v1::ShipClassSpec, ReconcileError> {
    ship_class
        .spec
        .as_ref()
        .ok_or_else(|| ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec".to_string()))
}

pub(super) fn ship_class_cpu_cores(ship_class: &ShipClass) -> Result<u64, ReconcileError> {
    ship_class_spec(ship_class)?
        .cpu
        .as_ref()
        .map(|cpu| cpu.cores)
        .ok_or_else(|| {
            ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec.cpu".to_string())
        })
}

pub(super) fn ship_class_architecture(ship_class: &ShipClass) -> Result<&str, ReconcileError> {
    ship_class_spec(ship_class)?
        .cpu
        .as_ref()
        .map(|cpu| cpu.architecture.as_str())
        .ok_or_else(|| {
            ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec.cpu".to_string())
        })
}

pub(super) fn ship_class_memory_size_string(
    ship_class: &ShipClass,
) -> Result<String, ReconcileError> {
    ship_class_spec(ship_class)?
        .memory
        .as_ref()
        .map(|memory| memory.size.clone())
        .ok_or_else(|| {
            ReconcileError::FieldMissing("v1.ShipClass".to_string(), "spec.memory".to_string())
        })
}

pub(super) fn ship_class_memory_bytes(ship_class: &ShipClass) -> Result<u64, ReconcileError> {
    let size = ship_class_memory_size_string(ship_class)?;
    parse_memory_size(&size).ok_or_else(|| {
        ReconcileError::Runtime(crate::runtime::error::RuntimeError::MemorySize(size))
    })
}

pub(super) fn parse_memory_size(value: &str) -> Option<u64> {
    SizedString(value.to_string()).as_byte_length()
}

pub(super) fn current_nic_ids(
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
            let mac = format!(
                "52:54:00:{:02x}:{:02x}:{:02x}",
                digest[0], digest[1], digest[2]
            );
            format!("nic-{}", sanitize_identifier(&mac))
        })
        .collect()
}

pub(super) fn current_volume_ids(
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

pub(super) fn pvc_aliases(spec: &ShipSpec) -> Result<Vec<String>, ReconcileError> {
    Ok(normalized_ship_volumes(spec)?
        .into_iter()
        .filter_map(|volume| match volume.source {
            NormalizedVolumeSource::PersistentVolumeClaim { .. } => Some(volume.name),
            _ => None,
        })
        .collect())
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
            service_account_name: None,
            automount_service_account_token: None,
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
        let mac = format!(
            "52:54:00:{:02x}:{:02x}:{:02x}",
            digest[0], digest[1], digest[2]
        );
        let expected = format!("nic-{}", sanitize_identifier(&mac));

        assert_eq!(ids[0], expected);
    }
}
