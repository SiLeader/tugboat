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
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipStatus};

impl ShipReconciler {
    pub(super) async fn reconcile(&self, event: WatchEvent<Ship>) -> Result<(), ReconcileError> {
        match event {
            WatchEvent::Modified(ship) => self.reconcile_modified(ship).await,
            WatchEvent::Deleted(ship) => self.reconcile_deleted(ship).await,
            WatchEvent::Added(ship) => self.reconcile_added(ship).await,
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
