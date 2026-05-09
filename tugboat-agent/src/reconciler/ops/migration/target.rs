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

use tracing::{error, info};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipSpec};
use tugboat_vm_runtime_interface::status::VmStatus;

use super::super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_READY};
use super::MigrationStateMachine;
use crate::reconciler::error::ReconcileError;

impl<'a> MigrationStateMachine<'a> {
    /// Reconcile a Ship whose `targetNodeName` matches the local node.
    ///
    /// Returns `true` when the Ship is being managed locally (either an active
    /// migration target or one whose target runtime has been recovered).
    pub(super) async fn try_reconcile_target_side(
        &self,
        ship: &Ship,
        ship_id: &str,
        ship_spec: &ShipSpec,
    ) -> Result<bool, ReconcileError> {
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
                    ReconcileError::FieldMissing("v1.Ship".to_string(), "metadata.name".to_string())
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

        Ok(has_local_runtime)
    }
}
