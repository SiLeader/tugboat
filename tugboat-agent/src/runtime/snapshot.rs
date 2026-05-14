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
use tugboat_vm_runtime_interface::snapshot::{
    VmSnapshotCreateRequest, VmSnapshotCreateResponse, VmSnapshotDeleteRequest,
    VmSnapshotListRequest, VmSnapshotListResponse, VmSnapshotMode, VmSnapshotRestoreRequest,
};

#[allow(dead_code)]
impl RuntimeOperator {
    pub(crate) async fn snapshot_create(
        &self,
        id: &str,
        mode: VmSnapshotMode,
    ) -> Result<VmSnapshotCreateResponse, RuntimeError> {
        Ok(self
            .operator
            .snapshot_create(VmSnapshotCreateRequest {
                ship_id: id.to_string(),
                mode,
            })
            .await?)
    }

    pub(crate) async fn snapshot_delete(&self, id: &str, handle: &str) -> Result<(), RuntimeError> {
        self.operator
            .snapshot_delete(VmSnapshotDeleteRequest {
                ship_id: id.to_string(),
                handle: handle.to_string(),
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn snapshot_restore(
        &self,
        id: &str,
        handle: &str,
    ) -> Result<(), RuntimeError> {
        self.operator
            .snapshot_restore(VmSnapshotRestoreRequest {
                ship_id: id.to_string(),
                handle: handle.to_string(),
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn snapshot_list(
        &self,
        id: &str,
    ) -> Result<VmSnapshotListResponse, RuntimeError> {
        Ok(self
            .operator
            .snapshot_list(VmSnapshotListRequest {
                ship_id: id.to_string(),
            })
            .await?)
    }
}
