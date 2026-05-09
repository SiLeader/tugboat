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
use crate::runtime::error::RuntimeError;
use async_trait::async_trait;
use std::collections::BTreeSet;
use tracing::warn;
use tugboat_client::{Api, WatchParams};
use tugboat_resources::NODE_RUNTIME_CLASS_LABEL_KEY;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    Node, RuntimeClass, Ship, ShipClass, ShipCondition, ShipMigrationStatus,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::migrate::{VmMigrationParams, VmMigrationStatusResponse};
use tugboat_vm_runtime_interface::status::VmStatus;

use super::{PHASE_COMPLETED, PHASE_FAILED};
mod preflight;
mod source;
mod target;
use preflight::{
    validate_storage_eligibility, validate_target_architecture, validate_target_network_capability,
    validate_target_node_readiness, validate_target_resource_capacity,
};

/// Maximum time (seconds) the source waits for the target to publish its
/// migration receiver before giving up.
const MIGRATION_PENDING_TIMEOUT_SECS: i64 = 120;

/// Maximum time (seconds) an in-progress QEMU memory transfer may run before
/// the source declares the migration failed and cancels it.
const MIGRATION_ACTIVE_TIMEOUT_SECS: i64 = 1800;

/// Returns `true` when the given timestamp is older than `timeout_secs` ago.
/// A missing timestamp is treated as not timed out so that very old status
/// records written before this feature was deployed are not immediately
/// cancelled.
fn is_migration_timed_out(timestamp: &Option<Time>, timeout_secs: i64) -> bool {
    let Some(ts) = timestamp else {
        return false;
    };
    let now = Time::now();
    // Treat far-future timestamps (e.g. from clock skew) as not timed out but
    // log a warning so operators can investigate.
    if ts.seconds > now.seconds {
        tracing::warn!(
            "Migration timestamp is {} seconds in the future; possible clock skew",
            ts.seconds - now.seconds
        );
        return false;
    }
    now.seconds - ts.seconds > timeout_secs
}

use super::upsert_ship_condition;

#[async_trait]
pub trait MigrationContext: Send + Sync {
    fn node_name(&self) -> &str;
    async fn has_ship(&self, ship_id: &str) -> bool;
    async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError>;
    async fn preflight_migration(
        &self,
        ship: &Ship,
        target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError>;
    async fn migrate(
        &self,
        ship_id: &str,
        target_address: String,
        target_port: u16,
        params: VmMigrationParams,
    ) -> Result<(), RuntimeError>;
    async fn check_migration_status(
        &self,
        ship_id: &str,
    ) -> Result<VmMigrationStatusResponse, RuntimeError>;
    async fn finish_source_migration(&self, ship_id: &str) -> Result<(), RuntimeError>;
    async fn cancel_migration(&self, ship_id: &str) -> Result<(), RuntimeError>;
    /// Resolve migration tuning parameters for the given Ship from its ShipClass.
    async fn resolve_migration_params(&self, ship: &Ship) -> VmMigrationParams;
    async fn local_runtime_status(&self, ship_id: &str) -> Result<Option<VmStatus>, RuntimeError>;

    async fn update_migration_status(
        &self,
        namespace: &str,
        name: &str,
        migration: ShipMigrationStatus,
        condition_status: &str,
        condition_message: String,
    ) -> Result<(), ReconcileError>;

    async fn patch_ship(
        &self,
        namespace: &str,
        name: &str,
        patch: serde_json::Value,
    ) -> Result<(), ReconcileError>;
}

#[async_trait]
impl MigrationContext for ShipReconciler {
    fn node_name(&self) -> &str {
        &self.node_name
    }
    async fn has_ship(&self, ship_id: &str) -> bool {
        self.runtime_operator.has_ship(ship_id).await
    }
    async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError> {
        self.reconcile_deleted(ship).await
    }
    async fn preflight_migration(
        &self,
        ship: &Ship,
        target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError> {
        ShipReconciler::preflight_migration(self, ship, target_node_name).await
    }
    async fn migrate(
        &self,
        ship_id: &str,
        target_address: String,
        target_port: u16,
        params: VmMigrationParams,
    ) -> Result<(), RuntimeError> {
        self.runtime_operator
            .migrate(ship_id, target_address, target_port, params)
            .await
    }
    async fn check_migration_status(
        &self,
        ship_id: &str,
    ) -> Result<VmMigrationStatusResponse, RuntimeError> {
        self.runtime_operator.check_migration_status(ship_id).await
    }
    async fn finish_source_migration(&self, ship_id: &str) -> Result<(), RuntimeError> {
        self.runtime_operator.finish_source_migration(ship_id).await
    }
    async fn cancel_migration(&self, ship_id: &str) -> Result<(), RuntimeError> {
        self.runtime_operator.cancel_migration(ship_id).await
    }
    async fn resolve_migration_params(&self, ship: &Ship) -> VmMigrationParams {
        let Some(ship_class_name) = ship.spec.as_ref().map(|s| s.ship_class.as_str()) else {
            return VmMigrationParams::default();
        };
        let ship_class = match self.ship_class_api.get(ship_class_name).await {
            Ok(Some(ship_class)) => ship_class,
            Ok(None) => {
                let ship_name = ship
                    .object_meta
                    .as_ref()
                    .and_then(|meta| meta.name.as_deref())
                    .unwrap_or("<unknown>");
                let ship_namespace = ship
                    .object_meta
                    .as_ref()
                    .and_then(|meta| meta.namespace.as_deref())
                    .unwrap_or("<unknown>");
                warn!(
                    "Falling back to default migration parameters for ship {ship_namespace}/{ship_name}: ShipClass '{ship_class_name}' was not found"
                );
                return VmMigrationParams::default();
            }
            Err(err) => {
                let ship_name = ship
                    .object_meta
                    .as_ref()
                    .and_then(|meta| meta.name.as_deref())
                    .unwrap_or("<unknown>");
                let ship_namespace = ship
                    .object_meta
                    .as_ref()
                    .and_then(|meta| meta.namespace.as_deref())
                    .unwrap_or("<unknown>");
                warn!(
                    "Falling back to default migration parameters for ship {ship_namespace}/{ship_name}: failed to fetch ShipClass '{ship_class_name}': {err}"
                );
                return VmMigrationParams::default();
            }
        };
        let Some(migration_spec) = ship_class.spec.as_ref().and_then(|s| s.migration.as_ref())
        else {
            return VmMigrationParams::default();
        };
        VmMigrationParams {
            max_bandwidth_bytes_per_sec: migration_spec.max_bandwidth_bytes_per_sec,
            downtime_limit_ms: migration_spec.downtime_limit_ms,
            xbzrle_cache_size_bytes: migration_spec.xbzrle_cache_size_bytes,
            postcopy_enabled: migration_spec.postcopy_enabled.unwrap_or(false),
        }
    }
    async fn local_runtime_status(&self, ship_id: &str) -> Result<Option<VmStatus>, RuntimeError> {
        self.runtime_operator.status(ship_id).await
    }

    async fn update_migration_status(
        &self,
        namespace: &str,
        name: &str,
        migration: ShipMigrationStatus,
        condition_status: &str,
        condition_message: String,
    ) -> Result<(), ReconcileError> {
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let mut conditions = api
            .get(name)
            .await?
            .and_then(|ship| ship.status)
            .map(|status| status.conditions)
            .unwrap_or_default();
        upsert_ship_condition(
            &mut conditions,
            ShipCondition {
                status: condition_status.to_string(),
                message: condition_message,
                timestamp: Some(Time::now()),
            },
        );
        let patch = serde_json::json!({
            "status": {
                "migration": migration,
                "conditions": conditions
            }
        });
        api.patch_status(name, patch).await?;
        Ok(())
    }

    async fn patch_ship(
        &self,
        namespace: &str,
        name: &str,
        patch: serde_json::Value,
    ) -> Result<(), ReconcileError> {
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        api.patch(name, patch).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationPreflight {
    Ready,
    Reject(String),
}

pub struct MigrationStateMachine<'a> {
    context: &'a dyn MigrationContext,
}

impl<'a> MigrationStateMachine<'a> {
    pub fn new(context: &'a dyn MigrationContext) -> Self {
        Self { context }
    }

    fn with_stats(
        &self,
        mut migration: ShipMigrationStatus,
        runtime_status: Option<&VmMigrationStatusResponse>,
    ) -> ShipMigrationStatus {
        migration.bytes_transferred = runtime_status.and_then(|status| status.bytes_transferred);
        migration.bytes_remaining = runtime_status.and_then(|status| status.bytes_remaining);
        migration
    }

    async fn mark_source_migration_failed(
        &self,
        namespace: &str,
        name: &str,
        target_node_name: String,
        target_address: Option<String>,
        target_port: Option<u32>,
        message: String,
    ) -> Result<(), ReconcileError> {
        self.context
            .update_migration_status(
                namespace,
                name,
                ShipMigrationStatus {
                    phase: PHASE_FAILED.to_string(),
                    source_node_name: Some(self.context.node_name().to_string()),
                    target_node_name: Some(target_node_name),
                    target_address,
                    target_port,
                    message: message.clone(),
                    timestamp: Some(Time::now()),
                    bytes_transferred: None,
                    bytes_remaining: None,
                },
                "VmMigrationFailed",
                message,
            )
            .await
    }

    /// Attempt to cancel any in-progress QEMU migration on the source VM.
    /// Errors are logged but not propagated — the migration is already being
    /// marked failed, and a cancel failure must not mask that status update.
    async fn best_effort_cancel(&self, ship_id: &str) {
        if let Err(err) = self.context.cancel_migration(ship_id).await {
            warn!(
                "Failed to cancel QEMU migration for ship '{}' (best-effort): {}",
                ship_id, err
            );
        }
    }

    async fn finalize_completed_source_migration(
        &self,
        ship_id: &str,
        namespace: &str,
        name: &str,
        target_node_name: String,
        target_address: String,
        target_port: u32,
    ) -> Result<(), ReconcileError> {
        self.context
            .update_migration_status(
                namespace,
                name,
                ShipMigrationStatus {
                    phase: PHASE_COMPLETED.to_string(),
                    source_node_name: Some(self.context.node_name().to_string()),
                    target_node_name: Some(target_node_name.clone()),
                    target_address: Some(target_address.clone()),
                    target_port: Some(target_port),
                    message:
                        "Live migration completed successfully with preserved guest NIC identity"
                            .to_string(),
                    timestamp: Some(Time::now()),
                    bytes_transferred: None,
                    bytes_remaining: None,
                },
                "VmMigrated",
                format!(
                    "VM migrated successfully to node '{target_node_name}' with deterministic bridge, interface, and MAC identity"
                ),
            )
            .await?;

        if self.context.has_ship(ship_id).await {
            self.context.finish_source_migration(ship_id).await?;
        }

        let spec_patch = serde_json::json!({
            "spec": {
                "nodeName": target_node_name,
                "targetNodeName": null,
            }
        });
        self.context.patch_ship(namespace, name, spec_patch).await?;
        Ok(())
    }

    async fn finalize_completed_target_migration(
        &self,
        namespace: &str,
        name: &str,
        source_node_name: Option<String>,
        target_address: Option<String>,
        target_port: Option<u32>,
    ) -> Result<(), ReconcileError> {
        let target_node_name = self.context.node_name().to_string();
        self.context
            .update_migration_status(
                namespace,
                name,
                ShipMigrationStatus {
                    phase: PHASE_COMPLETED.to_string(),
                    source_node_name,
                    target_node_name: Some(target_node_name.clone()),
                    target_address,
                    target_port,
                    message: "Target node recovered a completed live migration and finalized cutover"
                        .to_string(),
                    timestamp: Some(Time::now()),
                    bytes_transferred: None,
                    bytes_remaining: None,
                },
                "VmMigrated",
                format!(
                    "Recovered completed live migration on target node '{target_node_name}' after the source stopped reporting progress"
                ),
            )
            .await?;

        let spec_patch = serde_json::json!({
            "spec": {
                "nodeName": target_node_name,
                "targetNodeName": null,
            }
        });
        self.context.patch_ship(namespace, name, spec_patch).await?;
        Ok(())
    }

    pub async fn try_reconcile(&self, ship: &Ship, ship_id: &str) -> Result<bool, ReconcileError> {
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };
        let Some(target_node_name) = ship_spec.target_node_name.clone() else {
            return Ok(false);
        };

        if target_node_name == self.context.node_name() {
            return self
                .try_reconcile_target_side(ship, ship_id, ship_spec)
                .await;
        }

        if ship_spec.node_name.as_deref() != Some(self.context.node_name()) {
            return Ok(false);
        }

        self.try_reconcile_source_side(ship, ship_id, ship_spec, target_node_name)
            .await
    }
}

impl ShipReconciler {
    async fn preflight_migration(
        &self,
        ship: &Ship,
        target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError> {
        let Some(ship_spec) = ship.spec.as_ref() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };

        if ship_spec.node_name.as_deref() == Some(target_node_name) {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' is already hosting the ship"
            )));
        }

        let namespace = ship.namespace().unwrap_or("default");
        let node_api: Api<Node> = Api::all(self.client.clone());
        let Some(target_node) = node_api.get(target_node_name).await? else {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' was not found"
            )));
        };
        let node_runtime_class_name = target_node
            .object_meta
            .as_ref()
            .and_then(|meta| meta.labels.get(NODE_RUNTIME_CLASS_LABEL_KEY))
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(node_runtime_class_name) = node_runtime_class_name else {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' does not declare '{}' label",
                NODE_RUNTIME_CLASS_LABEL_KEY
            )));
        };

        let runtime_class_api: Api<RuntimeClass> = Api::all(self.client.clone());
        let Some(target_runtime_class) = runtime_class_api.get(node_runtime_class_name).await?
        else {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' references missing runtime class '{node_runtime_class_name}'"
            )));
        };
        let target_supports_live_migration = target_runtime_class
            .spec
            .as_ref()
            .map(|spec| spec.live_migration)
            .unwrap_or(false);
        if !target_supports_live_migration {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' runtime class '{node_runtime_class_name}' does not support live migration"
            )));
        }

        let Some(ship_class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
            return Err(ReconcileError::ShipClassNotFound(
                ship_spec.ship_class.clone(),
            ));
        };

        if let Some(reason) = validate_target_node_readiness(&target_node) {
            return Ok(MigrationPreflight::Reject(reason));
        }
        if let Some(reason) = validate_target_architecture(&ship_class, &target_node) {
            return Ok(MigrationPreflight::Reject(reason));
        }

        let network_classes = self
            .get_related_network_classes(namespace, ship_spec)
            .await?;
        if let Some(reason) = validate_target_network_capability(&target_node, &network_classes) {
            return Ok(MigrationPreflight::Reject(reason));
        }

        let volumes = self.get_related_volumes(namespace, ship_spec).await?;
        if let Some(reason) = validate_storage_eligibility(&volumes) {
            return Ok(MigrationPreflight::Reject(reason));
        }

        let ships = self
            .ship_all_api
            .list_with_params(
                &WatchParams::default().fields(format!("spec.nodeName={target_node_name}")),
            )
            .await?;
        let ship_classes = self
            .get_ship_classes_for_capacity_check(&ships, &ship_spec.ship_class)
            .await?;
        if let Some(reason) =
            validate_target_resource_capacity(&target_node, &ships, &ship_classes, &ship_class)
        {
            return Ok(MigrationPreflight::Reject(reason));
        }

        Ok(MigrationPreflight::Ready)
    }

    async fn get_ship_classes_for_capacity_check(
        &self,
        ships: &[Ship],
        requested_ship_class_name: &str,
    ) -> Result<Vec<ShipClass>, ReconcileError> {
        let mut class_names = BTreeSet::from([requested_ship_class_name.to_string()]);
        for ship in ships {
            if let Some(spec) = ship.spec.as_ref() {
                class_names.insert(spec.ship_class.clone());
            }
        }

        let mut ship_classes = Vec::with_capacity(class_names.len());
        for class_name in class_names {
            if let Some(ship_class) = self.ship_class_api.get(&class_name).await? {
                ship_classes.push(ship_class);
            }
        }

        Ok(ship_classes)
    }
}

#[cfg(test)]
mod tests;
