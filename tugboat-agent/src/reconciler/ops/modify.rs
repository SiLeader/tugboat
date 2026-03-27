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
use crate::reconciler::reconcile::AppendStatus;
use tracing::{info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition};
use tugboat_resources::manifests::meta::v1::Time;

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

        if !self.runtime_operator.has_ship(ship_id).await {
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
        let spec_fingerprint = super::spec_fingerprint(ship_spec)?;
        if self
            .runtime_operator
            .matches_spec(ship_id, &spec_fingerprint)
            .await
        {
            info!("Ship modified but desired spec is unchanged: {}", ship_id);
            return Ok(());
        }

        warn!(
            "Ship '{}' spec changed while running; live mutation is not supported, setting condition",
            ship_id
        );
        let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
        let mut status_ship = ship.clone();
        let (status, message) = if ship_spec.volume_claim_ref.is_empty() {
            (
                "SpecChangeRequiresRecreate",
                "Ship spec changed while running; live mutation is not supported — recreate the ship to apply the new spec".to_string(),
            )
        } else {
            (
                "CsiVolumeChangeRequiresRecreate",
                "Ship spec changed while running; CSI-backed volume attach, detach, and resize changes are not reconciled live — recreate the ship to apply the new storage plan".to_string(),
            )
        };
        status_ship.append_status(ShipCondition {
            status: status.to_string(),
            message,
            timestamp: Some(Time::now()),
        });
        api.replace_status(name, status_ship).await?;
        Ok(())
    }
}
