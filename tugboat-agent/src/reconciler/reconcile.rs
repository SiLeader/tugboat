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
use tugboat_client::Api;
use tugboat_client::runtime::{Action, FinalizerEvent, ReconcileEvent, finalizer};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipStatus};

const AGENT_FINALIZER: &str = "tugboat/agent";

impl ShipReconciler {
    pub(super) async fn reconcile(
        &self,
        event: ReconcileEvent<Ship>,
    ) -> Result<Action, ReconcileError> {
        match event {
            ReconcileEvent::Applied(ship) => self.reconcile_applied(ship).await,
            ReconcileEvent::Deleted(ship) => self.reconcile_deleted_event(ship).await,
        }
    }

    async fn reconcile_applied(&self, ship: Ship) -> Result<Action, ReconcileError> {
        let api = self.ship_api(&ship);
        finalizer(&api, AGENT_FINALIZER, ship, {
            let this = self.clone();
            move |event| async move {
                match event {
                    FinalizerEvent::Apply(ship) => this.reconcile_active(ship).await,
                    FinalizerEvent::Cleanup(ship) => this.reconcile_cleanup(ship).await,
                }
            }
        })
        .await
        .map_err(|err| ReconcileError::Finalizer(err.to_string()))
    }

    async fn reconcile_active(&self, ship: Ship) -> Result<Action, ReconcileError> {
        // Delegate to reconcile_modified which already handles the case where the
        // ship does not yet exist in the runtime (falling back to reconcile_added).
        // This avoids a TOCTOU race between a separate has_ship() check and the
        // subsequent reconcile call.
        self.reconcile_modified(ship).await?;
        Ok(Action::await_change())
    }

    async fn reconcile_cleanup(&self, ship: Ship) -> Result<Action, ReconcileError> {
        self.reconcile_deleted(ship).await?;
        Ok(Action::await_change())
    }

    async fn reconcile_deleted_event(&self, ship: Ship) -> Result<Action, ReconcileError> {
        // Keep a best-effort post-delete cleanup path for ships that reached deletion
        // before the agent finalizer had a chance to protect them.
        self.reconcile_deleted(ship).await?;
        Ok(Action::await_change())
    }

    fn ship_api(&self, ship: &Ship) -> Api<Ship> {
        Api::namespaced(self.client.clone(), ship.namespace().unwrap_or("default"))
    }
}

pub(super) trait AppendStatus {
    fn append_status(&mut self, condition: ShipCondition);
}

impl AppendStatus for ShipStatus {
    fn append_status(&mut self, condition: ShipCondition) {
        if let Some(existing) = self
            .conditions
            .iter_mut()
            .find(|c| c.status == condition.status)
        {
            if existing.message == condition.message {
                return;
            }
            *existing = condition;
        } else {
            self.conditions.push(condition);
        }
    }
}

impl AppendStatus for Ship {
    fn append_status(&mut self, condition: ShipCondition) {
        match &mut self.status {
            None => {
                self.status = Some(ShipStatus {
                    conditions: vec![condition],
                    ..Default::default()
                });
            }
            Some(status) => {
                status.append_status(condition);
            }
        }
    }
}
