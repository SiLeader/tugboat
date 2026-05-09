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

use tracing::warn;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipMigrationStatus, ShipSpec};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::migrate::VmMigrationPhase;

use super::super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};
use super::{
    MIGRATION_ACTIVE_TIMEOUT_SECS, MIGRATION_PENDING_TIMEOUT_SECS, MigrationPreflight,
    MigrationStateMachine, is_migration_timed_out,
};
use crate::reconciler::error::ReconcileError;
use crate::runtime::error::RuntimeError;

impl<'a> MigrationStateMachine<'a> {
    /// Reconcile a Ship whose `nodeName` is the local node and that has a
    /// `targetNodeName` set (i.e. this node is the migration source).
    ///
    /// Returns `true` once the migration has produced any state (preflight
    /// rejection, pending, in-flight, or completed).
    pub(super) async fn try_reconcile_source_side(
        &self,
        ship: &Ship,
        ship_id: &str,
        ship_spec: &ShipSpec,
        target_node_name: String,
    ) -> Result<bool, ReconcileError> {
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
            return self
                .handle_source_initial_phase(ship, ship_spec, namespace, name, target_node_name)
                .await;
        };

        match migration_status.phase.as_str() {
            PHASE_PENDING => {
                self.handle_source_pending_phase(
                    ship_id,
                    namespace,
                    name,
                    target_node_name,
                    &migration_status,
                )
                .await
            }
            PHASE_READY => {
                self.handle_source_ready_phase(
                    ship,
                    ship_id,
                    namespace,
                    name,
                    target_node_name,
                    migration_status,
                )
                .await
            }
            PHASE_MIGRATING => {
                self.handle_source_migrating_phase(
                    ship_id,
                    namespace,
                    name,
                    target_node_name,
                    migration_status,
                )
                .await
            }
            PHASE_COMPLETED => {
                self.handle_source_completed_phase(
                    ship_id,
                    ship_spec,
                    namespace,
                    name,
                    target_node_name,
                    migration_status,
                )
                .await
            }
            // PHASE_FAILED or any unknown terminal phase.
            _ => Ok(true),
        }
    }

    async fn handle_source_initial_phase(
        &self,
        ship: &Ship,
        ship_spec: &ShipSpec,
        namespace: &str,
        name: &str,
        target_node_name: String,
    ) -> Result<bool, ReconcileError> {
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
                    message: "Waiting for target node to prepare migration receiver".to_string(),
                    timestamp: Some(Time::now()),
                    bytes_transferred: None,
                    bytes_remaining: None,
                },
                "VmMigrationPending",
                "Waiting for target node to prepare migration receiver".to_string(),
            )
            .await?;
        Ok(true)
    }

    async fn handle_source_pending_phase(
        &self,
        ship_id: &str,
        namespace: &str,
        name: &str,
        target_node_name: String,
        migration_status: &ShipMigrationStatus,
    ) -> Result<bool, ReconcileError> {
        if is_migration_timed_out(&migration_status.timestamp, MIGRATION_PENDING_TIMEOUT_SECS) {
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

    async fn handle_source_ready_phase(
        &self,
        ship: &Ship,
        ship_id: &str,
        namespace: &str,
        name: &str,
        target_node_name: String,
        migration_status: ShipMigrationStatus,
    ) -> Result<bool, ReconcileError> {
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
                                source_node_name: Some(self.context.node_name().to_string()),
                                target_node_name: Some(target_node_name),
                                target_address: Some(target_address),
                                target_port: Some(target_port),
                                message: "Live migration is already in progress".to_string(),
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

    async fn handle_source_migrating_phase(
        &self,
        ship_id: &str,
        namespace: &str,
        name: &str,
        target_node_name: String,
        migration_status: ShipMigrationStatus,
    ) -> Result<bool, ReconcileError> {
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
                                source_node_name: Some(self.context.node_name().to_string()),
                                target_node_name: Some(target_node_name.clone()),
                                target_address: Some(target_address.clone()),
                                target_port: Some(target_port),
                                message: runtime_status.message.clone(),
                                timestamp: migration_status.timestamp,
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

    async fn handle_source_completed_phase(
        &self,
        ship_id: &str,
        ship_spec: &ShipSpec,
        namespace: &str,
        name: &str,
        target_node_name: String,
        migration_status: ShipMigrationStatus,
    ) -> Result<bool, ReconcileError> {
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
}
