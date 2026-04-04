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

use crate::cni::NetworkClassInfo;
use crate::csi::READ_WRITE_MANY;
use crate::node_registration::NODE_ARCH_LABEL;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::VolumeInfo;
use crate::runtime::error::RuntimeError;
use async_trait::async_trait;
use std::collections::BTreeSet;
use tracing::{error, info, warn};
use tugboat_client::{Api, WatchParams};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    Node, NodeCniPluginStatus, Ship, ShipClass, ShipCondition, ShipMigrationStatus,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::migrate::{
    VmMigrationParams, VmMigrationPhase, VmMigrationStatusResponse,
};
use tugboat_vm_runtime_interface::status::VmStatus;

use super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};

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
    now.seconds.saturating_sub(ts.seconds) > timeout_secs
}

fn upsert_ship_condition(conditions: &mut Vec<ShipCondition>, condition: ShipCondition) {
    if let Some(existing) = conditions
        .iter_mut()
        .find(|existing| existing.status == condition.status)
    {
        *existing = condition;
    } else {
        conditions.push(condition);
    }
}

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
        let Ok(Some(ship_class)) = self.ship_class_api.get(ship_class_name).await else {
            return VmMigrationParams::default();
        };
        let Some(migration_spec) = ship_class.spec.as_ref().and_then(|s| s.migration.as_ref())
        else {
            return VmMigrationParams::default();
        };
        VmMigrationParams {
            max_bandwidth_bytes_per_sec: migration_spec.max_bandwidth_bytes_per_sec,
            downtime_limit_ms: migration_spec.downtime_limit_ms,
            xbzrle_cache_size_bytes: migration_spec.xbzrle_cache_size_bytes,
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
            let has_local_runtime = self.context.has_ship(ship_id).await;
            if let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && migration.phase == PHASE_FAILED
            {
                if has_local_runtime {
                    info!(
                        "Migration failed for ship '{}', cleaning up incoming VM on target node",
                        ship_id
                    );
                    if let Err(err) = self.context.reconcile_deleted(ship.clone()).await {
                        error!(
                            "Failed to clean up incoming VM for failed migration '{}': {}",
                            ship_id, err
                        );
                    }
                }
                return Ok(true);
            }

            if has_local_runtime
                && let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && matches!(
                    migration.phase.as_str(),
                    PHASE_READY | PHASE_MIGRATING | PHASE_COMPLETED
                )
                && let Some(VmStatus::Running) = self.context.local_runtime_status(ship_id).await?
            {
                info!(
                    "Recovered active migration target for ship '{}', finalizing cutover on target node",
                    ship_id
                );
                self.finalize_completed_target_migration(
                    ship.namespace().unwrap_or("default"),
                    ship.name().ok_or_else(|| {
                        ReconcileError::FieldMissing(
                            "v1.Ship".to_string(),
                            "metadata.name".to_string(),
                        )
                    })?,
                    migration
                        .source_node_name
                        .clone()
                        .or_else(|| ship_spec.node_name.clone()),
                    migration.target_address.clone(),
                    migration.target_port,
                )
                .await?;
            }

            return Ok(has_local_runtime);
        }

        if ship_spec.node_name.as_deref() != Some(self.context.node_name()) {
            return Ok(false);
        }

        let Some(name) = ship.name() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship.namespace().unwrap_or("default");
        let migration_status = ship
            .status
            .as_ref()
            .and_then(|status| status.migration.clone());

        let Some(migration_status) = migration_status else {
            match self
                .context
                .preflight_migration(ship, &target_node_name)
                .await?
            {
                MigrationPreflight::Ready => {}
                MigrationPreflight::Reject(reason) => {
                    let message = format!(
                        "Migration preflight failed: {reason}. Source VM remains on the source node."
                    );
                    self.context
                        .update_migration_status(
                            namespace,
                            name,
                            ShipMigrationStatus {
                                phase: PHASE_FAILED.to_string(),
                                source_node_name: ship_spec.node_name.clone(),
                                target_node_name: Some(target_node_name),
                                target_address: None,
                                target_port: None,
                                message: message.clone(),
                                timestamp: Some(Time::now()),
                                bytes_transferred: None,
                                bytes_remaining: None,
                            },
                            "VmMigrationPreflightFailed",
                            message,
                        )
                        .await?;
                    return Ok(true);
                }
            }
            self.context
                .update_migration_status(
                    namespace,
                    name,
                    ShipMigrationStatus {
                        phase: PHASE_PENDING.to_string(),
                        source_node_name: ship_spec.node_name.clone(),
                        target_node_name: Some(target_node_name),
                        target_address: None,
                        target_port: None,
                        message: "Waiting for target node to prepare migration receiver"
                            .to_string(),
                        timestamp: Some(Time::now()),
                        bytes_transferred: None,
                        bytes_remaining: None,
                    },
                    "VmMigrationPending",
                    "Waiting for target node to prepare migration receiver".to_string(),
                )
                .await?;
            return Ok(true);
        };

        match migration_status.phase.as_str() {
            PHASE_PENDING => {
                if is_migration_timed_out(
                    &migration_status.timestamp,
                    MIGRATION_PENDING_TIMEOUT_SECS,
                ) {
                    let message = format!(
                        "Target node '{target_node_name}' did not become ready within {} seconds. \
                         Source VM remains authoritative.",
                        MIGRATION_PENDING_TIMEOUT_SECS
                    );
                    self.best_effort_cancel(ship_id).await;
                    self.mark_source_migration_failed(
                        namespace,
                        name,
                        target_node_name,
                        None,
                        None,
                        message,
                    )
                    .await?;
                }
                Ok(true)
            }
            PHASE_READY => {
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };

                match self.context.check_migration_status(ship_id).await {
                    Ok(status) if matches!(status.phase, VmMigrationPhase::None) => {
                        if is_migration_timed_out(
                            &migration_status.timestamp,
                            MIGRATION_PENDING_TIMEOUT_SECS,
                        ) {
                            let message = format!(
                                "Migration receiver on target node '{target_node_name}' became \
                                 ready but source timed out before starting transfer. \
                                 Source VM remains authoritative."
                            );
                            self.best_effort_cancel(ship_id).await;
                            self.mark_source_migration_failed(
                                namespace,
                                name,
                                target_node_name,
                                Some(target_address),
                                Some(target_port),
                                message,
                            )
                            .await?;
                            return Ok(true);
                        }
                        let migration_params = self.context.resolve_migration_params(ship).await;
                        if let Err(err) = self
                            .context
                            .migrate(
                                ship_id,
                                target_address.clone(),
                                target_port as u16,
                                migration_params,
                            )
                            .await
                        {
                            let message = format!("Failed to start live migration: {err}");
                            self.best_effort_cancel(ship_id).await;
                            self.mark_source_migration_failed(
                                namespace,
                                name,
                                target_node_name,
                                Some(target_address),
                                Some(target_port),
                                message.clone(),
                            )
                            .await?;
                            return Err(err.into());
                        }

                        self.context
                            .update_migration_status(
                                namespace,
                                name,
                                ShipMigrationStatus {
                                    phase: PHASE_MIGRATING.to_string(),
                                    source_node_name: Some(self.context.node_name().to_string()),
                                    target_node_name: Some(target_node_name),
                                    target_address: Some(target_address),
                                    target_port: Some(target_port),
                                    message: "Live migration in progress".to_string(),
                                    timestamp: Some(Time::now()),
                                    bytes_transferred: None,
                                    bytes_remaining: None,
                                },
                                "VmMigrating",
                                "Live migration in progress".to_string(),
                            )
                            .await?;
                        Ok(true)
                    }
                    Ok(status)
                        if matches!(
                            status.phase,
                            VmMigrationPhase::Setup | VmMigrationPhase::Active
                        ) =>
                    {
                        self.context
                            .update_migration_status(
                                namespace,
                                name,
                                self.with_stats(
                                    ShipMigrationStatus {
                                        phase: PHASE_MIGRATING.to_string(),
                                        source_node_name: Some(
                                            self.context.node_name().to_string(),
                                        ),
                                        target_node_name: Some(target_node_name),
                                        target_address: Some(target_address),
                                        target_port: Some(target_port),
                                        message: "Live migration is already in progress"
                                            .to_string(),
                                        timestamp: Some(Time::now()),
                                        bytes_transferred: None,
                                        bytes_remaining: None,
                                    },
                                    Some(&status),
                                ),
                                "VmMigrating",
                                "Live migration is already in progress".to_string(),
                            )
                            .await?;
                        Ok(true)
                    }
                    Ok(status) if matches!(status.phase, VmMigrationPhase::Completed) => {
                        self.finalize_completed_source_migration(
                            ship_id,
                            namespace,
                            name,
                            target_node_name,
                            target_address,
                            target_port,
                        )
                        .await?;
                        Ok(true)
                    }
                    Ok(status)
                        if matches!(
                            status.phase,
                            VmMigrationPhase::Failed | VmMigrationPhase::Cancelled
                        ) =>
                    {
                        let message = format!(
                            "Live migration did not complete (phase: {:?}). Source VM remains authoritative; clean up the target and retry when ready.",
                            status.phase
                        );
                        self.best_effort_cancel(ship_id).await;
                        self.mark_source_migration_failed(
                            namespace,
                            name,
                            target_node_name,
                            Some(target_address),
                            Some(target_port),
                            message,
                        )
                        .await?;
                        Ok(true)
                    }
                    Ok(_) => Ok(true),
                    Err(err) => {
                        warn!(
                            "Failed to check migration status before starting migration for ship '{}': {}",
                            ship_id, err
                        );
                        Ok(true)
                    }
                }
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

                let runtime_status = match self.context.check_migration_status(ship_id).await {
                    Ok(status) => status,
                    Err(err) => {
                        warn!(
                            "Failed to check migration status for ship '{}': {}",
                            ship_id, err
                        );
                        return Ok(true); // retry on next event
                    }
                };

                match &runtime_status.phase {
                    VmMigrationPhase::Completed => {
                        self.finalize_completed_source_migration(
                            ship_id,
                            namespace,
                            name,
                            target_node_name,
                            target_address,
                            target_port,
                        )
                        .await?;
                        Ok(true)
                    }
                    VmMigrationPhase::Failed | VmMigrationPhase::Cancelled => {
                        let message = format!(
                            "Live migration did not complete (phase: {:?}). Source VM remains authoritative; clean up the target and retry when ready.",
                            runtime_status.phase
                        );
                        self.best_effort_cancel(ship_id).await;
                        self.mark_source_migration_failed(
                            namespace,
                            name,
                            target_node_name,
                            Some(target_address),
                            Some(target_port),
                            message.clone(),
                        )
                        .await?;
                        Err(ReconcileError::Runtime(RuntimeError::MigrationFailed(
                            message,
                        )))
                    }
                    _ => {
                        self.context
                            .update_migration_status(
                                namespace,
                                name,
                                self.with_stats(
                                    ShipMigrationStatus {
                                        phase: PHASE_MIGRATING.to_string(),
                                        source_node_name: Some(
                                            self.context.node_name().to_string(),
                                        ),
                                        target_node_name: Some(target_node_name.clone()),
                                        target_address: Some(target_address.clone()),
                                        target_port: Some(target_port),
                                        message: runtime_status.message.clone(),
                                        timestamp: migration_status.timestamp.clone(),
                                        bytes_transferred: None,
                                        bytes_remaining: None,
                                    },
                                    Some(&runtime_status),
                                ),
                                "VmMigrating",
                                "Live migration in progress".to_string(),
                            )
                            .await?;
                        // Still active (Setup, Active, None); check for timeout.
                        if is_migration_timed_out(
                            &migration_status.timestamp,
                            MIGRATION_ACTIVE_TIMEOUT_SECS,
                        ) {
                            let message = format!(
                                "Live migration exceeded the maximum allowed time ({} seconds). \
                                 Cancelling and leaving source VM authoritative.",
                                MIGRATION_ACTIVE_TIMEOUT_SECS
                            );
                            self.best_effort_cancel(ship_id).await;
                            self.mark_source_migration_failed(
                                namespace,
                                name,
                                target_node_name,
                                Some(target_address),
                                Some(target_port),
                                message.clone(),
                            )
                            .await?;
                            return Err(ReconcileError::Runtime(RuntimeError::MigrationFailed(
                                message,
                            )));
                        }
                        Ok(true)
                    }
                }
            }
            PHASE_COMPLETED => {
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

                if ship_spec.node_name.as_deref() == Some(self.context.node_name()) {
                    self.finalize_completed_source_migration(
                        ship_id,
                        namespace,
                        name,
                        target_node_name,
                        target_address,
                        target_port,
                    )
                    .await?;
                }
                Ok(true)
            }
            // PHASE_FAILED or any unknown terminal phase.
            _ => Ok(true),
        }
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

fn validate_target_node_readiness(target_node: &Node) -> Option<String> {
    let Some(meta) = target_node.object_meta.as_ref() else {
        return Some("target node is missing metadata".to_string());
    };
    let node_name = meta.name.as_deref().unwrap_or("<unknown>");
    let Some(spec) = target_node.spec.as_ref() else {
        return Some(format!("target node '{node_name}' is missing spec"));
    };

    if !spec.ips.iter().any(|ip| {
        ip.parse::<std::net::IpAddr>()
            .map(|addr| !addr.is_loopback())
            .unwrap_or(false)
    }) {
        return Some(format!(
            "target node '{node_name}' does not advertise a reachable non-loopback IP"
        ));
    }

    let Some(status) = target_node.status.as_ref() else {
        return Some(format!("target node '{node_name}' has no published status"));
    };
    let Some(condition) = status
        .conditions
        .iter()
        .find(|condition| condition.r#type == "CniReady")
    else {
        return Some(format!(
            "target node '{node_name}' does not publish a CniReady condition"
        ));
    };

    if condition.status == "True" {
        None
    } else if condition.message.is_empty() {
        Some(format!("target node '{node_name}' is not CNI-ready"))
    } else {
        Some(format!(
            "target node '{node_name}' is not CNI-ready: {}",
            condition.message
        ))
    }
}

fn validate_target_architecture(ship_class: &ShipClass, target_node: &Node) -> Option<String> {
    let requested = ship_class
        .spec
        .as_ref()
        .and_then(|spec| spec.cpu.as_ref())
        .map(|cpu| normalize_architecture(&cpu.architecture))
        .filter(|arch| !arch.is_empty())?;
    let meta = target_node.object_meta.as_ref()?;
    let node_name = meta.name.as_deref().unwrap_or("<unknown>");
    let Some(actual) = meta
        .labels
        .get(NODE_ARCH_LABEL)
        .map(|arch| normalize_architecture(arch))
    else {
        return Some(format!(
            "target node '{node_name}' does not advertise '{}' label",
            NODE_ARCH_LABEL
        ));
    };

    if requested == actual {
        None
    } else {
        Some(format!(
            "target node '{node_name}' architecture '{actual}' is incompatible with ship class architecture '{requested}'"
        ))
    }
}

fn validate_target_network_capability(
    target_node: &Node,
    network_classes: &[NetworkClassInfo],
) -> Option<String> {
    let statuses = target_node
        .status
        .as_ref()
        .map(|status| status.cni_plugins.as_slice())
        .unwrap_or(&[]);
    let mut required_plugins = BTreeSet::from(["loopback".to_string()]);

    for network_class in network_classes {
        let plugin = normalized_plugin(&network_class.spec);
        match plugin {
            "bridge" => {
                required_plugins.insert("bridge".to_string());
            }
            "flannel" => {
                required_plugins.insert("bridge".to_string());
                required_plugins.insert("flannel".to_string());
                if network_class
                    .spec
                    .flannel
                    .as_ref()
                    .and_then(|flannel| flannel.port_mappings)
                    .unwrap_or(false)
                {
                    required_plugins.insert("portmap".to_string());
                }
            }
            other => {
                return Some(format!(
                    "network class '{}' requires unsupported cniPlugin '{}'",
                    network_class.name, other
                ));
            }
        }
    }

    for plugin in required_plugins {
        if let Some(reason) = require_plugin_ready(statuses, &plugin) {
            return Some(reason);
        }
    }

    None
}

fn validate_storage_eligibility(volumes: &[VolumeInfo]) -> Option<String> {
    for volume in volumes {
        let Some(volume) = volume.persistent_volume_claim() else {
            continue;
        };

        let claim_supports_rwx = volume
            .claim
            .access_modes
            .iter()
            .any(|mode| mode == READ_WRITE_MANY);
        let pv_supports_rwx = volume
            .volume
            .access_modes
            .iter()
            .any(|mode| mode == READ_WRITE_MANY);

        if !claim_supports_rwx || !pv_supports_rwx {
            return Some(format!(
                "persistent volume claim '{}' must use shared storage with '{}' access on both the claim and persistent volume for live migration",
                volume.claim_name, READ_WRITE_MANY
            ));
        }
    }

    None
}

fn validate_target_resource_capacity(
    target_node: &Node,
    ships: &[Ship],
    ship_classes: &[ShipClass],
    requested_ship_class: &ShipClass,
) -> Option<String> {
    let node_name = target_node
        .object_meta
        .as_ref()
        .and_then(|meta| meta.name.as_deref())
        .unwrap_or("<unknown>");

    let (alloc_cpu, alloc_memory) = node_allocatable(target_node);
    let (used_cpu, used_memory) = node_resource_usage(ships, ship_classes, node_name);
    let (req_cpu, req_memory) = ship_class_requested_resources(requested_ship_class);

    let avail_cpu = alloc_cpu.saturating_sub(used_cpu);
    if req_cpu > avail_cpu {
        return Some(format!(
            "target node '{node_name}' has insufficient CPU for live migration: requested={req_cpu}, available={avail_cpu}"
        ));
    }

    let avail_memory = alloc_memory.saturating_sub(used_memory);
    if req_memory > avail_memory {
        return Some(format!(
            "target node '{node_name}' has insufficient memory for live migration: requested={req_memory}, available={avail_memory}"
        ));
    }

    None
}

fn node_allocatable(node: &Node) -> (u64, u64) {
    let spec = node.spec.as_ref();
    let resource = spec.and_then(|s| s.resource.as_ref());
    let overcommit = spec.and_then(|s| s.overcommit.as_ref());

    let base_cpu = resource.map(|r| r.cpu).unwrap_or(0);
    let base_memory = resource.map(|r| r.memory).unwrap_or(0);

    let cpu_ratio: f64 = overcommit
        .and_then(|o| o.cpu_ratio.parse().ok())
        .unwrap_or(1.0);
    let memory_ratio: f64 = overcommit
        .and_then(|o| o.memory_ratio.parse().ok())
        .unwrap_or(1.0);

    let alloc_cpu = (base_cpu as f64 * cpu_ratio) as u64;
    let alloc_memory = (base_memory as f64 * memory_ratio) as u64;

    (alloc_cpu, alloc_memory)
}

fn node_resource_usage(ships: &[Ship], ship_classes: &[ShipClass], node_name: &str) -> (u64, u64) {
    let mut cpu_used: u64 = 0;
    let mut memory_used: u64 = 0;

    for ship in ships {
        let assigned_node = ship
            .spec
            .as_ref()
            .and_then(|spec| spec.node_name.as_deref());
        if assigned_node != Some(node_name) {
            continue;
        }

        let class_name = ship
            .spec
            .as_ref()
            .map(|spec| spec.ship_class.as_str())
            .unwrap_or("");
        let Some(ship_class) = find_ship_class(ship_classes, class_name) else {
            continue;
        };
        let (cpu, memory) = ship_class_requested_resources(ship_class);
        cpu_used = cpu_used.saturating_add(cpu);
        memory_used = memory_used.saturating_add(memory);
    }

    (cpu_used, memory_used)
}

fn find_ship_class<'a>(ship_classes: &'a [ShipClass], name: &str) -> Option<&'a ShipClass> {
    ship_classes.iter().find(|ship_class| {
        ship_class
            .object_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            == Some(name)
    })
}

fn ship_class_requested_resources(ship_class: &ShipClass) -> (u64, u64) {
    let spec = ship_class.spec.as_ref();
    let cpu = spec
        .and_then(|spec| spec.cpu.as_ref())
        .map(|cpu| cpu.cores)
        .unwrap_or(0);
    let memory = spec
        .and_then(|spec| spec.memory.as_ref())
        .map(|memory| parse_memory_size(&memory.size))
        .unwrap_or(0);
    (cpu, memory)
}

fn parse_memory_size(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    if let Ok(bytes) = s.parse::<u64>() {
        return bytes;
    }

    let (num_str, suffix) = if let Some(n) = s.strip_suffix("Gi") {
        (n, "Gi")
    } else if let Some(n) = s.strip_suffix("Mi") {
        (n, "Mi")
    } else if let Some(n) = s.strip_suffix("Ki") {
        (n, "Ki")
    } else if let Some(n) = s.strip_suffix("Ti") {
        (n, "Ti")
    } else if let Some(n) = s.strip_suffix('G') {
        (n, "G")
    } else if let Some(n) = s.strip_suffix('M') {
        (n, "M")
    } else if let Some(n) = s.strip_suffix('K') {
        (n, "K")
    } else if let Some(n) = s.strip_suffix('T') {
        (n, "T")
    } else {
        return 0;
    };

    let Ok(num) = num_str.parse::<f64>() else {
        return 0;
    };

    let multiplier: u64 = match suffix {
        "Ki" => 1024,
        "Mi" => 1024 * 1024,
        "Gi" => 1024 * 1024 * 1024,
        "Ti" => 1024 * 1024 * 1024 * 1024,
        "K" => 1000,
        "M" => 1000 * 1000,
        "G" => 1000 * 1000 * 1000,
        "T" => 1000 * 1000 * 1000 * 1000,
        _ => return 0,
    };

    let result = num * multiplier as f64;
    if result >= u64::MAX as f64 {
        return u64::MAX;
    }

    result as u64
}

fn normalize_architecture(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => "amd64".to_string(),
        "aarch64" | "arm64" => "arm64".to_string(),
        other => other.to_string(),
    }
}

fn normalized_plugin(spec: &tugboat_resources::manifests::core::v1::NetworkClassSpec) -> &str {
    let plugin = spec.cni_plugin.trim();
    if plugin.is_empty() { "bridge" } else { plugin }
}

fn require_plugin_ready(statuses: &[NodeCniPluginStatus], plugin: &str) -> Option<String> {
    let Some(status) = statuses.iter().find(|status| status.name == plugin) else {
        return Some(format!(
            "target node does not advertise required CNI plugin '{}'",
            plugin
        ));
    };

    if status.ready.unwrap_or(false) {
        None
    } else if status.message.is_empty() {
        Some(format!("required CNI plugin '{}' is not ready", plugin))
    } else {
        Some(format!(
            "required CNI plugin '{}' is not ready: {}",
            plugin, status.message
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tugboat_resources::manifests::core::v1::{
        CpuSpec, CsiPersistentVolumeSource, MemorySpec, NodeOvercommitSpec, NodeResource, NodeSpec,
        PersistentVolumeClaimSpec, PersistentVolumeSpec, ShipClassSpec, ShipSpec, ShipStatus,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[derive(Debug, Clone)]
    struct StatusUpdate {
        migration: ShipMigrationStatus,
        condition_status: String,
        condition_message: String,
    }

    struct FakeContext {
        node_name: String,
        has_ship: bool,
        migration_phase: VmMigrationPhase,
        runtime_status: Option<VmStatus>,
        preflight: MigrationPreflight,
        updates: Mutex<Vec<StatusUpdate>>,
        migrate_calls: Mutex<Vec<(String, u16)>>,
        finish_calls: Mutex<usize>,
        cancel_calls: Mutex<usize>,
        ship_patches: Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait]
    impl MigrationContext for FakeContext {
        fn node_name(&self) -> &str {
            &self.node_name
        }
        async fn has_ship(&self, _ship_id: &str) -> bool {
            self.has_ship
        }
        async fn reconcile_deleted(&self, _ship: Ship) -> Result<(), ReconcileError> {
            Ok(())
        }
        async fn preflight_migration(
            &self,
            _ship: &Ship,
            _target_node_name: &str,
        ) -> Result<MigrationPreflight, ReconcileError> {
            Ok(self.preflight.clone())
        }
        async fn migrate(
            &self,
            _ship_id: &str,
            addr: String,
            port: u16,
            _params: VmMigrationParams,
        ) -> Result<(), RuntimeError> {
            self.migrate_calls.lock().unwrap().push((addr, port));
            Ok(())
        }
        async fn resolve_migration_params(&self, _ship: &Ship) -> VmMigrationParams {
            VmMigrationParams::default()
        }
        async fn check_migration_status(
            &self,
            _ship_id: &str,
        ) -> Result<VmMigrationStatusResponse, RuntimeError> {
            Ok(VmMigrationStatusResponse {
                phase: self.migration_phase.clone(),
                message: format!("{:?}", &self.migration_phase),
                bytes_transferred: None,
                bytes_remaining: None,
                ram_dirty_rate_mbps: None,
            })
        }
        async fn finish_source_migration(&self, _ship_id: &str) -> Result<(), RuntimeError> {
            *self.finish_calls.lock().unwrap() += 1;
            Ok(())
        }
        async fn cancel_migration(&self, _ship_id: &str) -> Result<(), RuntimeError> {
            *self.cancel_calls.lock().unwrap() += 1;
            Ok(())
        }
        async fn local_runtime_status(
            &self,
            _ship_id: &str,
        ) -> Result<Option<VmStatus>, RuntimeError> {
            Ok(self.runtime_status)
        }

        async fn update_migration_status(
            &self,
            _namespace: &str,
            _name: &str,
            migration: ShipMigrationStatus,
            condition_status: &str,
            condition_message: String,
        ) -> Result<(), ReconcileError> {
            self.updates.lock().unwrap().push(StatusUpdate {
                migration,
                condition_status: condition_status.to_string(),
                condition_message,
            });
            Ok(())
        }

        async fn patch_ship(
            &self,
            _namespace: &str,
            _name: &str,
            patch: serde_json::Value,
        ) -> Result<(), ReconcileError> {
            self.ship_patches.lock().unwrap().push(patch);
            Ok(())
        }
    }

    fn fake_context(node_name: &str, migration_phase: VmMigrationPhase) -> FakeContext {
        FakeContext {
            node_name: node_name.to_string(),
            has_ship: true,
            migration_phase,
            runtime_status: Some(VmStatus::Paused),
            preflight: MigrationPreflight::Ready,
            updates: Mutex::new(Vec::new()),
            migrate_calls: Mutex::new(Vec::new()),
            finish_calls: Mutex::new(0),
            cancel_calls: Mutex::new(0),
            ship_patches: Mutex::new(Vec::new()),
        }
    }

    fn stale_timestamp(age_secs: i64) -> Time {
        let now = Time::now();
        Time {
            seconds: now.seconds - age_secs,
            nanos: now.nanos,
        }
    }

    #[tokio::test]
    async fn test_migration_not_for_me() {
        let context = fake_context("node-1", VmMigrationPhase::Completed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-2".to_string()),
                target_node_name: Some("node-3".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_migration_source_initial() {
        let context = fake_context("node-1", VmMigrationPhase::Completed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Should initiate migration (update status to Pending) and return true
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_migration_source_ready() {
        let context = fake_context("node-1", VmMigrationPhase::None);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_READY.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(context.migrate_calls.lock().unwrap().len(), 1);
        assert_eq!(
            context
                .updates
                .lock()
                .unwrap()
                .last()
                .map(|update| update.migration.phase.as_str()),
            Some(PHASE_MIGRATING)
        );
        assert!(
            context
                .updates
                .lock()
                .unwrap()
                .last()
                .and_then(|update| update.migration.timestamp.as_ref())
                .is_some()
        );
    }

    #[tokio::test]
    async fn test_migration_source_migrating_completed() {
        let context = fake_context("node-1", VmMigrationPhase::Completed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.finish_calls.lock().unwrap(), 1);
        assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_migration_target_failed_cleanup() {
        let context = fake_context("target-node", VmMigrationPhase::Failed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("source-node".to_string()),
                target_node_name: Some("target-node".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_FAILED.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Should handle cleanup on target node and return true
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_migration_preflight_rejection_marks_ship_failed() {
        let context = FakeContext {
            preflight: MigrationPreflight::Reject("target node is not CNI-ready".to_string()),
            ..fake_context("node-1", VmMigrationPhase::Completed)
        };
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);

        let updates = context.updates.lock().unwrap();
        let update = updates.last().expect("expected migration status update");
        assert_eq!(update.migration.phase, PHASE_FAILED);
        assert_eq!(update.condition_status, "VmMigrationPreflightFailed");
        assert!(
            update
                .condition_message
                .contains("Source VM remains on the source node")
        );
    }

    #[tokio::test]
    async fn test_migration_source_ready_detects_existing_active_migration() {
        let context = fake_context("node-1", VmMigrationPhase::Active);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_READY.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert!(context.migrate_calls.lock().unwrap().is_empty());
        assert_eq!(
            context
                .updates
                .lock()
                .unwrap()
                .last()
                .map(|update| update.migration.phase.as_str()),
            Some(PHASE_MIGRATING)
        );
    }

    #[tokio::test]
    async fn test_migration_completed_without_runtime_still_finalizes_cutover() {
        let context = FakeContext {
            has_ship: false,
            ..fake_context("node-1", VmMigrationPhase::Completed)
        };
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_COMPLETED.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.finish_calls.lock().unwrap(), 0);
        assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_running_target_runtime_finalizes_cutover_after_source_loss() {
        let context = FakeContext {
            node_name: "target-node".to_string(),
            runtime_status: Some(VmStatus::Running),
            ..fake_context("target-node", VmMigrationPhase::None)
        };
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("source-node".to_string()),
                target_node_name: Some("target-node".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    source_node_name: Some("source-node".to_string()),
                    target_node_name: Some("target-node".to_string()),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.finish_calls.lock().unwrap(), 0);
        assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
        assert_eq!(
            context
                .updates
                .lock()
                .unwrap()
                .last()
                .map(|update| update.migration.phase.as_str()),
            Some(PHASE_COMPLETED)
        );
    }

    #[tokio::test]
    async fn test_pending_timeout_marks_failed_and_cancels() {
        let context = fake_context("node-1", VmMigrationPhase::None);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_PENDING.to_string(),
                    timestamp: Some(stale_timestamp(MIGRATION_PENDING_TIMEOUT_SECS + 10)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.cancel_calls.lock().unwrap(), 1);
        let updates = context.updates.lock().unwrap();
        assert_eq!(
            updates.last().map(|u| u.migration.phase.as_str()),
            Some(PHASE_FAILED)
        );
        assert!(
            updates
                .last()
                .unwrap()
                .condition_message
                .contains("did not become ready")
        );
    }

    #[tokio::test]
    async fn test_no_timeout_when_pending_timestamp_is_recent() {
        let context = fake_context("node-1", VmMigrationPhase::None);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_PENDING.to_string(),
                    timestamp: Some(Time::now()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.cancel_calls.lock().unwrap(), 0);
        assert!(context.updates.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_migrating_timeout_marks_failed_and_cancels() {
        let context = fake_context("node-1", VmMigrationPhase::Active);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    timestamp: Some(stale_timestamp(MIGRATION_ACTIVE_TIMEOUT_SECS + 10)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await;
        // Timeout returns an Err (MigrationFailed)
        assert!(result.is_err());
        assert_eq!(*context.cancel_calls.lock().unwrap(), 1);
        let updates = context.updates.lock().unwrap();
        assert_eq!(
            updates.last().map(|u| u.migration.phase.as_str()),
            Some(PHASE_FAILED)
        );
        assert!(
            updates
                .last()
                .unwrap()
                .condition_message
                .contains("exceeded the maximum allowed time")
        );
    }

    #[tokio::test]
    async fn test_migrating_status_update_preserves_existing_timestamp() {
        let context = fake_context("node-1", VmMigrationPhase::Active);
        let sm = MigrationStateMachine::new(&context);
        let timestamp = stale_timestamp(120);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    timestamp: Some(timestamp),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);

        let updates = context.updates.lock().unwrap();
        let update = updates.last().expect("expected migration status update");
        assert_eq!(update.migration.phase, PHASE_MIGRATING);
        assert_eq!(update.migration.timestamp, Some(timestamp));
    }

    #[tokio::test]
    async fn test_cancel_called_when_qemu_reports_failed_during_migrating() {
        let context = fake_context("node-1", VmMigrationPhase::Failed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    timestamp: Some(Time::now()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await;
        assert!(result.is_err());
        assert_eq!(*context.cancel_calls.lock().unwrap(), 1);
    }

    #[test]
    fn normalizes_common_architecture_aliases() {
        assert_eq!(normalize_architecture("x86_64"), "amd64");
        assert_eq!(normalize_architecture("amd64"), "amd64");
        assert_eq!(normalize_architecture("aarch64"), "arm64");
        assert_eq!(normalize_architecture("arm64"), "arm64");
    }

    #[test]
    fn rejects_non_shared_persistent_volumes_for_live_migration() {
        let volume = VolumeInfo::PersistentVolumeClaim(Box::new(
            crate::reconciler::volume::PersistentVolumeClaimVolumeInfo {
                name: "data".to_string(),
                claim_name: "data-pvc".to_string(),
                volume_name: "data-pv".to_string(),
                claim: PersistentVolumeClaimSpec {
                    access_modes: vec!["ReadWriteOnce".to_string()],
                    ..Default::default()
                },
                volume: PersistentVolumeSpec {
                    access_modes: vec!["ReadWriteOnce".to_string()],
                    csi: Some(CsiPersistentVolumeSource::default()),
                    ..Default::default()
                },
                status: None,
                source: CsiPersistentVolumeSource::default(),
            },
        ));

        let reason =
            validate_storage_eligibility(&[volume]).expect("non-shared storage should be rejected");
        assert!(reason.contains(READ_WRITE_MANY));
    }

    #[test]
    fn rejects_target_node_without_enough_cpu_capacity_for_live_migration() {
        let target_node = Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-2".to_string()),
                ..Default::default()
            }),
            spec: Some(NodeSpec {
                overcommit: Some(NodeOvercommitSpec {
                    cpu_ratio: "1".to_string(),
                    memory_ratio: "1".to_string(),
                }),
                resource: Some(NodeResource {
                    cpu: 4,
                    memory: 8 * 1024 * 1024 * 1024,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let incumbent_class = ShipClass {
            object_meta: Some(ObjectMeta {
                name: Some("incumbent".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipClassSpec {
                cpu: Some(CpuSpec {
                    architecture: "amd64".to_string(),
                    cores: 3,
                }),
                memory: Some(MemorySpec {
                    size: "2Gi".to_string(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let migrating_class = ShipClass {
            object_meta: Some(ObjectMeta {
                name: Some("migrating".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipClassSpec {
                cpu: Some(CpuSpec {
                    architecture: "amd64".to_string(),
                    cores: 2,
                }),
                memory: Some(MemorySpec {
                    size: "1Gi".to_string(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let resident_ship = Ship {
            spec: Some(ShipSpec {
                node_name: Some("node-2".to_string()),
                ship_class: "incumbent".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let reason = validate_target_resource_capacity(
            &target_node,
            &[resident_ship],
            &[incumbent_class, migrating_class.clone()],
            &migrating_class,
        )
        .expect("insufficient CPU capacity should be rejected");

        assert!(reason.contains("insufficient CPU"));
        assert!(reason.contains("requested=2"));
        assert!(reason.contains("available=1"));
    }

    #[test]
    fn upsert_ship_condition_preserves_unrelated_conditions() {
        let timestamp = Time::now();
        let mut conditions = vec![
            ShipCondition {
                status: "Running".to_string(),
                message: "VM is running".to_string(),
                timestamp: Some(timestamp),
            },
            ShipCondition {
                status: "VmMigrationPending".to_string(),
                message: "old migration state".to_string(),
                timestamp: Some(timestamp),
            },
        ];

        upsert_ship_condition(
            &mut conditions,
            ShipCondition {
                status: "VmMigrationPending".to_string(),
                message: "new migration state".to_string(),
                timestamp: Some(Time::now()),
            },
        );

        assert_eq!(conditions.len(), 2);
        assert!(conditions.iter().any(|condition| {
            condition.status == "Running" && condition.message == "VM is running"
        }));
        assert!(conditions.iter().any(|condition| {
            condition.status == "VmMigrationPending" && condition.message == "new migration state"
        }));
    }
}
