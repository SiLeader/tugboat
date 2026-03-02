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
use tracing::{info, warn};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::Ship;

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

        if !self.runtime_operator.has_ship(ship_id).await {
            info!(
                "Ship modified but not running, treating as added: {}",
                ship_id
            );
            return self.reconcile_added(ship).await;
        }

        // TODO modified
        warn!("Modify Ship is not handled yet. (Ship: {ship:?})");
        Ok(())
    }
}
