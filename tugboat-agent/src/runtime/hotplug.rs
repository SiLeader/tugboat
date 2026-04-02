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
use tracing::info;
use tugboat_vm_runtime_interface::hotplug::VmHotplugRequest;

impl RuntimeOperator {
    pub(crate) async fn hotplug_resources(
        &self,
        id: &str,
        new_spec_fingerprint: String,
        cpu_cores: u64,
        memory_size: u64,
    ) -> Result<(), RuntimeError> {
        let Some(current) = self.spec_state(id).await else {
            return Err(RuntimeError::HotplugNotSupported(format!(
                "runtime state for ship '{id}' is missing"
            )));
        };

        if cpu_cores < current.cpu_cores {
            return Err(RuntimeError::HotplugNotSupported(format!(
                "CPU decrease from {} to {} is not supported",
                current.cpu_cores, cpu_cores
            )));
        }
        if memory_size < current.memory_size {
            return Err(RuntimeError::HotplugNotSupported(format!(
                "memory decrease from {} to {} is not supported",
                current.memory_size, memory_size
            )));
        }

        let cpu_to_add = cpu_cores - current.cpu_cores;
        let memory_to_add = memory_size - current.memory_size;

        if cpu_to_add > 0 || memory_to_add > 0 {
            self.operator
                .hotplug(VmHotplugRequest {
                    id: id.to_string(),
                    // send absolute desired totals (breaking change of semantics)
                    vcpus_to_add: (cpu_cores > 0).then_some(cpu_cores),
                    size_bytes_to_add: (memory_size > 0).then_some(memory_size),
                    // include current observed values so runtime can compute the delta safely
                    current_vcpus: Some(current.cpu_cores),
                    current_size_bytes: Some(current.memory_size),
                })
                .await?;
        }

        if let Some(runtime) = self.children.write().await.get_mut(id) {
            runtime.spec_state_mut().cpu_cores = cpu_cores;
            runtime.spec_state_mut().memory_size = memory_size;
            runtime.update_spec_fingerprint(new_spec_fingerprint);
        }

        info!(
            "Applied hotplug to ship '{}': +{} vCPU(s), +{} bytes memory",
            id, cpu_to_add, memory_to_add
        );
        Ok(())
    }
}
