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

use crate::execute::vm::qemu::QemuVm;
use crate::execute::vm::{RunVm, Spawner};
use serde::Deserialize;
use tugboat_vm_runtime_interface::run::VmRunRequest;

#[derive(Debug, Clone)]
pub struct QemuVmBuilder {
    config: QemuVmConfig,
}

impl QemuVmBuilder {
    pub fn new(config: QemuVmConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Spawner for QemuVmBuilder {
    async fn spawn(&self, args: VmRunRequest) -> crate::Result<()> {
        QemuVm::new(&self.config, args).run_vm().await
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QemuVmConfig {
    pub executables: QemuVmConfigExecutables,
    pub disk_image_location: String,
    pub kvm: QemuVmConfigKvm,
    pub uefi: Option<QemuVmConfigUefi>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QemuVmConfigExecutables {
    pub qemu: String,
    pub qemu_img: String,
    pub ip: String,
    pub tc: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QemuVmConfigKvm {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QemuVmConfigUefi {
    pub code_file: String,
    pub vars_file: String,
}

impl QemuVmConfig {
    pub fn get_uds_path(&self, id: &str) -> String {
        format!("{}/{id}.qmp.sock", self.disk_image_location)
    }

    pub fn get_uds_url(&self, id: &str) -> String {
        format!("unix:{}", self.get_uds_path(id))
    }
}
