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

use crate::runtime::error::RuntimeError;
use crate::runtime::inner::Runtime;
use tugboat_resources::manifests::core::v1::ShipCondition;
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::operator::VmRuntimeOperator;
use tugboat_vm_runtime_interface::status::{VmStatus, VmStatusResponse};

pub(crate) struct RuntimeStatusChecker {
    pub id: String,
    pub namespace: String,
    pub ship_name: String,
}

impl RuntimeStatusChecker {
    pub(crate) async fn check(
        &self,
        operator: &VmRuntimeOperator,
    ) -> Result<ShipCondition, RuntimeError> {
        let status = operator.status(&self.id).await?;
        Ok(status_to_condition(status))
    }
}

impl Runtime {
    pub(crate) fn status(&self) -> RuntimeStatusChecker {
        RuntimeStatusChecker {
            id: self.id.clone(),
            namespace: self.namespace.clone(),
            ship_name: self.ship_name.clone(),
        }
    }
}

fn status_to_condition(status: VmStatusResponse) -> ShipCondition {
    let code = match status.status {
        VmStatus::Paused => "Paused",
        VmStatus::Running => "Running",
        VmStatus::Shutdown => "Shutdown",
        VmStatus::Suspended => "Suspended",
        VmStatus::Panicked => "Panicked",
        VmStatus::Error => "Error",
        VmStatus::Prelaunch => "Prelaunch",
    };
    ShipCondition {
        status: code.to_string(),
        message: status.message,
        timestamp: Some(Time::now()),
    }
}
