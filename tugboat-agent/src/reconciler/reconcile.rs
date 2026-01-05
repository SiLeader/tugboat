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
use tugboat_client::WatchEvent;
use tugboat_resources::manifests::core::v1::Ship;

impl ShipReconciler {
    pub(super) async fn reconcile(&self, event: WatchEvent<Ship>) -> Result<(), ReconcileError> {
        match event {
            WatchEvent::Added(ship) => {
                let Some(ship_spec) = &ship.spec else {
                    return Err(ReconcileError::FieldMissing(
                        "v1.Ship".to_string(),
                        "spec".to_string(),
                    ));
                };
                let Some(class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
                    return Err(ReconcileError::ShipClassNotFound(ship_spec.ship_class));
                };

                self.runtime_operator.run(ship, class).await?;
                Ok(())
            }
            WatchEvent::Modified(_ship) => {
                todo!()
            }
            WatchEvent::Deleted(_ship) => {
                todo!()
            }
        }
    }
}
