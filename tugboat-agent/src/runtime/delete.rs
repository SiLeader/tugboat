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
use tracing::{debug, info};
use tugboat_vm_runtime_interface::stop::{VmStopRequest, VmStopType};

impl RuntimeOperator {
    pub(crate) async fn delete(&self, id: String) -> Result<(), RuntimeError> {
        debug!("Delete VM: {}", id);
        let req = VmStopRequest {
            id: id.clone(),
            stop_type: VmStopType::Shutdown,
        };

        match self.operator.stop(req).await {
            Ok(_) => info!("VM '{}' stopped successfully", id),
            Err(e) => return Err(e.into()),
        }

        let mut children = self.children.write().await;
        children.remove(&id);

        Ok(())
    }
}
