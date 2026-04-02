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
use tracing::{error, info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipMigrationStatus};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::migrate::VmMigrationPhase;

use super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};

#[async_trait]
pub trait MigrationContext: Send + Sync {
    fn node_name(&self) -> &str;
    async fn has_ship(&self, ship_id: &str) -> bool;
    async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError>;
    async fn migrate(
        &self,
        ship_id: &str,
        target_address: String,
        target_port: u16,
    ) -> Result<(), RuntimeError>;
    async fn check_migration_status(&self, ship_id: &str) -> Result<VmMigrationPhase, RuntimeError>;
    async fn finish_source_migration(&self, ship_id: &str) -> Result<(), RuntimeError>;

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

    async fn patch_ship_status(
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
    async fn migrate(
        &self,
        ship_id: &str,
        target_address: String,
        target_port: u16,
    ) -> Result<(), RuntimeError> {
        self.runtime_operator
            .migrate(ship_id, target_address, target_port)
            .await
    }
    async fn check_migration_status(&self, ship_id: &str) -> Result<VmMigrationPhase, RuntimeError> {
        self.runtime_operator.check_migration_status(ship_id).await
    }
    async fn finish_source_migration(&self, ship_id: &str) -> Result<(), RuntimeError> {
        self.runtime_operator.finish_source_migration(ship_id).await
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

    async fn patch_ship_status(
        &self,
        namespace: &str,
        name: &str,
        patch: serde_json::Value,
    ) -> Result<(), ReconcileError> {
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        api.patch_status(name, patch).await?;
        Ok(())
    }
}

pub struct MigrationStateMachine<'a> {
    context: &'a dyn MigrationContext,
}

impl<'a> MigrationStateMachine<'a> {
    pub fn new(context: &'a dyn MigrationContext) -> Self {
        Self { context }
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
            if let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && migration.phase == PHASE_FAILED
            {
                if self.context.has_ship(ship_id).await {
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
            return Ok(self.context.has_ship(ship_id).await);
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
                    },
                    "VmMigrationPending",
                    "Waiting for target node to prepare migration receiver".to_string(),
                )
                .await?;
            return Ok(true);
        };

        match migration_status.phase.as_str() {
            PHASE_PENDING => Ok(true),
            PHASE_READY => {
                // Target is ready; issue the non-blocking QMP migrate command.
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };

                if let Err(err) = self
                    .context
                    .migrate(ship_id, target_address.clone(), target_port as u16)
                    .await
                {
                    self.context
                        .update_migration_status(
                            namespace,
                            name,
                            ShipMigrationStatus {
                                phase: PHASE_FAILED.to_string(),
                                source_node_name: Some(self.context.node_name().to_string()),
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

                let phase = match self.context.check_migration_status(ship_id).await {
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
                        let spec_patch = serde_json::json!({
                            "spec": {
                                "nodeName": target_node_name,
                                "targetNodeName": null,
                            }
                        });
                        self.context.patch_ship(namespace, name, spec_patch).await?;

                        let status_patch = serde_json::json!({
                            "status": {
                                "migration": {
                                    "phase": PHASE_COMPLETED,
                                    "sourceNodeName": self.context.node_name(),
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
                        self.context
                            .patch_ship_status(namespace, name, status_patch)
                            .await?;

                        if let Err(err) = self.context.finish_source_migration(ship_id).await {
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
                        self.context
                            .update_migration_status(
                                namespace,
                                name,
                                ShipMigrationStatus {
                                    phase: PHASE_FAILED.to_string(),
                                    source_node_name: Some(self.context.node_name().to_string()),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{ShipSpec, ShipStatus};
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    struct FakeContext {
        node_name: String,
        migration_phase: VmMigrationPhase,
    }

    #[async_trait]
    impl MigrationContext for FakeContext {
        fn node_name(&self) -> &str {
            &self.node_name
        }
        async fn has_ship(&self, _ship_id: &str) -> bool {
            true
        }
        async fn reconcile_deleted(&self, _ship: Ship) -> Result<(), ReconcileError> {
            Ok(())
        }
        async fn migrate(
            &self,
            _ship_id: &str,
            _addr: String,
            _port: u16,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }
        async fn check_migration_status(
            &self,
            _ship_id: &str,
        ) -> Result<VmMigrationPhase, RuntimeError> {
            Ok(self.migration_phase.clone())
        }
        async fn finish_source_migration(&self, _ship_id: &str) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn update_migration_status(
            &self,
            _namespace: &str,
            _name: &str,
            _migration: ShipMigrationStatus,
            _condition_status: &str,
            _condition_message: String,
        ) -> Result<(), ReconcileError> {
            Ok(())
        }

        async fn patch_ship(
            &self,
            _namespace: &str,
            _name: &str,
            _patch: serde_json::Value,
        ) -> Result<(), ReconcileError> {
            Ok(())
        }

        async fn patch_ship_status(
            &self,
            _namespace: &str,
            _name: &str,
            _patch: serde_json::Value,
        ) -> Result<(), ReconcileError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_migration_not_for_me() {
        let context = FakeContext {
            node_name: "node-1".to_string(),
            migration_phase: VmMigrationPhase::Completed,
        };
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
        let context = FakeContext {
            node_name: "node-1".to_string(),
            migration_phase: VmMigrationPhase::Completed,
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
            ..Default::default()
        };
        // Should initiate migration (update status to Pending) and return true
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_migration_source_ready() {
        let context = FakeContext {
            node_name: "node-1".to_string(),
            migration_phase: VmMigrationPhase::Completed,
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
    }

    #[tokio::test]
    async fn test_migration_source_migrating_completed() {
        let context = FakeContext {
            node_name: "node-1".to_string(),
            migration_phase: VmMigrationPhase::Completed,
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
    }

    #[tokio::test]
    async fn test_migration_target_failed_cleanup() {
        let context = FakeContext {
            node_name: "target-node".to_string(),
            migration_phase: VmMigrationPhase::Failed,
        };
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
}
