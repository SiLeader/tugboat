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
use tugboat_client::{Api, WatchEvent};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipStatus};
use tugboat_resources::manifests::meta::v1::Time;

impl ShipReconciler {
    pub(super) async fn reconcile(&self, event: WatchEvent<Ship>) -> Result<(), ReconcileError> {
        match event {
            WatchEvent::Added(ship) => {
                let Some(ship_metadata) = ship.object_meta() else {
                    return Err(ReconcileError::FieldMissing(
                        "v1.Ship".to_string(),
                        "metadata".to_string(),
                    ));
                };
                let Some(name) = &ship_metadata.name else {
                    return Err(ReconcileError::FieldMissing(
                        "v1.Ship".to_string(),
                        "metadata.name".to_string(),
                    ));
                };
                let Some(ship_id) = &ship_metadata.uid else {
                    return Err(ReconcileError::FieldMissing(
                        "v1.Ship".to_string(),
                        "metadata.uid".to_string(),
                    ));
                };
                let Some(ship_spec) = &ship.spec else {
                    return Err(ReconcileError::FieldMissing(
                        "v1.Ship".to_string(),
                        "spec".to_string(),
                    ));
                };
                let Some(class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
                    return Err(ReconcileError::ShipClassNotFound(
                        ship_spec.ship_class.clone(),
                    ));
                };
                let namespace = ship_metadata
                    .namespace
                    .clone()
                    .unwrap_or("default".to_string());

                let network_classes = self
                    .get_related_network_classes(&namespace, ship_spec)
                    .await?;

                {
                    let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
                    let mut status_ship = ship.clone();
                    status_ship.append_status(ShipCondition {
                        status: "VmCreating".to_string(),
                        message: "Creating new Virtual Machine".to_string(),
                        timestamp: Some(Time::now()),
                    });
                    api.replace_status(name, status_ship).await?;
                }

                let networks = self.cni.add(ship_id, network_classes).await?;
                self.runtime_operator
                    .create(ship_id, networks.as_slice())
                    .await?;
                self.runtime_operator.start(ship, class, networks).await?;
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

pub(super) trait AppendStatus {
    fn append_status(&mut self, condition: ShipCondition);
}

impl AppendStatus for ShipStatus {
    fn append_status(&mut self, condition: ShipCondition) {
        self.conditions.push(condition);
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
