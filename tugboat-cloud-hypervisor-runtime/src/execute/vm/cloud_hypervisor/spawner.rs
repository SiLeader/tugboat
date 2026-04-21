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

use crate::CloudHypervisorVmConfig;
use crate::execute::vm::cloud_hypervisor::CloudHypervisorVm;
use crate::execute::vm::{RunVm, Spawner};
use tugboat_vm_runtime_interface::run::VmRunRequest;

#[derive(Debug, Clone)]
pub struct CloudHypervisorVmBuilder {
    config: CloudHypervisorVmConfig,
}

impl CloudHypervisorVmBuilder {
    pub fn new(config: CloudHypervisorVmConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Spawner for CloudHypervisorVmBuilder {
    async fn spawn(&self, args: VmRunRequest) -> crate::Result<()> {
        CloudHypervisorVm::new(&self.config, args).run_vm().await
    }
}
