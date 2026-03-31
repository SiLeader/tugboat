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
        let pvc_vol_fp = super::pvc_volume_fingerprint(ship_spec)?;
        let mat_vol_fp = super::materialized_volume_fingerprint(ship_spec)?;

        let spec_changed = !self
            .runtime_operator
            .matches_spec_fingerprint(ship_id, &spec_fp)
            .await;
        let pvc_changed = !self
            .runtime_operator
            .matches_pvc_volume_fingerprint(ship_id, &pvc_vol_fp)
            .await;
        let mat_changed = !self
            .runtime_operator
            .matches_materialized_volume_fingerprint(ship_id, &mat_vol_fp)
            .await;

        if !spec_changed && !pvc_changed && !mat_changed {
            debug!("Ship '{}' runtime-significant spec is unchanged", ship_id);
            // Even though the spec is unchanged, check for pending volume expansions
            // since PV status updates are external to the Ship resource.
            self.check_pending_volume_expansions(ship_id, &namespace, ship_spec)
                .await;
            return Ok(());
        }

        if spec_changed || pvc_changed {
            if spec_changed {
                info!(
                    "Ship '{}' VM spec changed (image/class/network/uefi); recreating",
                    ship_id
                );
            }
            if pvc_changed {
                info!(
                    "Ship '{}' PVC volume references changed; recreating",
                    ship_id
                );
            }
            return self.reconcile_recreate(ship).await;
        }

        // Only materialized volumes (ConfigMap / Secret) changed — refresh in-place.
        debug!(
            "Ship '{}' materialized volumes changed; refreshing in-place",
            ship_id
        );
        self.refresh_materialized_volumes(ship_id, &namespace, ship_spec)
            .await?;
        self.runtime_operator
            .update_materialized_volume_fingerprint(ship_id, mat_vol_fp)
            .await;
        {
            let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
            let mut status_ship = ship.clone();
            status_ship.append_status(ShipCondition {
                status: "MaterializedVolumesRefreshed".to_string(),
                message: "Materialized volumes (ConfigMap/Secret) refreshed in-place".to_string(),
                timestamp: Some(Time::now()),
            });
            api.replace_status(name, status_ship).await?;
        }
        // Check volume expansions even when only materialized volumes changed.
        self.check_pending_volume_expansions(ship_id, &namespace, ship_spec)
            .await;
        Ok(())
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

    /// Re-materialize all ConfigMap / Secret volumes for a running ship without
    /// stopping the VM.  The files are written to the host directory that is
    /// already shared into the guest via virtio-9p, so the guest sees the
    /// updated content through the existing mount.
    async fn refresh_materialized_volumes(
        &self,
        ship_id: &str,
        namespace: &str,
        ship_spec: &ShipSpec,
    ) -> Result<(), ReconcileError> {
        let volumes = self.get_related_volumes(namespace, ship_spec).await?;
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
