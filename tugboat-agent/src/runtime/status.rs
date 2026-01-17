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

use crate::runtime::RuntimeOperator;
use crate::runtime::error::RuntimeError;
use tugboat_resources::manifests::core::v1::ShipCondition;

pub(crate) struct ShipConditionWithId {
    pub namespace: String,
    pub id: String,
    pub condition: ShipCondition,
}

impl RuntimeOperator {
    pub(crate) async fn collect_status(&self) -> Vec<Result<ShipConditionWithId, RuntimeError>> {
        let ships = self
            .children
            .read()
            .await
            .values()
            .map(|s| s.status())
            .collect::<Vec<_>>();

        let mut result = Vec::new();
        for ship in ships {
            let status = ship.check(&self.operator).await;
            result.push(status.map(|s| ShipConditionWithId {
                namespace: ship.namespace,
                id: ship.id,
                condition: s,
            }))
        }

        result
    }
}
