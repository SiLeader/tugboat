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
use tracing::{debug, info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipSpec};
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

        let spec_fp = super::spec_fingerprint(ship_spec)?;
        let vol_fp = super::volume_claims_fingerprint(ship_spec)?;

        let spec_changed = !self
            .runtime_operator
            .matches_spec_fingerprint(ship_id, &spec_fp)
            .await;
        let volumes_changed = !self
            .runtime_operator
            .matches_volume_fingerprint(ship_id, &vol_fp)
            .await;

        if !spec_changed && !volumes_changed {
            debug!("Ship '{}' runtime-significant spec is unchanged", ship_id);
            // Even though the spec is unchanged, check for pending volume expansions
            // since PV status updates are external to the Ship resource.
            self.check_pending_volume_expansions(ship_id, &namespace, ship_spec)
                .await;
            return Ok(());
        }

        // At least one runtime-significant field changed — report conditions.
        let mut conditions = Vec::new();
        if spec_changed {
            conditions.push(ShipCondition {
                status: "SpecChangeRequiresRecreate".to_string(),
                message: "Ship spec changed while running; live mutation is not supported — \
                    recreate the ship to apply the new spec"
                    .to_string(),
                timestamp: Some(Time::now()),
            });
        }
        if volumes_changed {
            conditions.push(ShipCondition {
                status: "CsiVolumeChangeRequiresRecreate".to_string(),
                message: "Ship volume claims changed while running; CSI-backed volume attach, \
                    detach, and resize changes are not reconciled live — recreate the ship \
                    to apply the new storage plan"
                    .to_string(),
                timestamp: Some(Time::now()),
            });
        }

        warn!(
            "Ship '{}' spec changed while running; live mutation is not supported, \
             setting condition(s)",
            ship_id
        );
        let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
        let mut status_ship = ship.clone();
        for condition in conditions {
            status_ship.append_status(condition);
        }
        api.replace_status(name, status_ship).await?;
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
        let published_volumes = match self.csi.load_published_volumes(ship_id) {
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
