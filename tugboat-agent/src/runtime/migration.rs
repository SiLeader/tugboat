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
use tugboat_vm_runtime_interface::migrate::{VmMigrateRequest, VmMigrationPhase};
use tugboat_vm_runtime_interface::status::VmStatus;
use tugboat_vm_runtime_interface::stop::{VmStopRequest, VmStopType};

impl RuntimeOperator {
    pub(crate) async fn migrate(
        &self,
        id: &str,
        destination_address: String,
        destination_port: u16,
    ) -> Result<(), RuntimeError> {
        self.operator
            .migrate(VmMigrateRequest {
                id: id.to_string(),
                destination_address,
                destination_port,
            })
            .await?;
        Ok(())
    }

    /// Check the current migration phase once without blocking.
    /// The caller is responsible for polling on subsequent reconcile events.
    pub(crate) async fn check_migration_status(
        &self,
        id: &str,
    ) -> Result<VmMigrationPhase, RuntimeError> {
        let status = self.operator.migration_status(id).await?;
        Ok(status.phase)
    }

    pub(crate) async fn status(&self, id: &str) -> Result<Option<VmStatus>, RuntimeError> {
        match self.operator.status(id).await {
            Ok(status) => Ok(Some(status.status)),
            Err(err) if super::delete::runtime_is_absent(&err) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub(crate) async fn finish_source_migration(&self, id: &str) -> Result<(), RuntimeError> {
        // VmStopType::PowerOff maps to QMP `quit`, which gracefully terminates
        // the QEMU process. The source VM is in a paused postmigrate state at
        // this point, so no guest-visible disruption occurs.
        self.operator
            .stop(VmStopRequest {
                id: id.to_string(),
                stop_type: VmStopType::PowerOff,
            })
            .await?;
        self.children.write().await.remove(id);
        Ok(())
    }
}
