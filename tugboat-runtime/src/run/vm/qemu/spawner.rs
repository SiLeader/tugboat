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

use crate::run::config::VmConfig;
use crate::run::vm::qemu::{QemuVm, SizeInBytes};
use crate::run::vm::{RunVm, Spawner};
use serde::Deserialize;

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
    async fn spawn(&self, args: VmConfig) -> crate::Result<()> {
        QemuVm::new(
            &self.config,
            args.image,
            args.cpu,
            SizeInBytes(args.memory),
            args.id,
        )
        .run_vm()
        .await
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QemuVmConfig {
    pub(crate) executables: QemuVmConfigExecutables,
    pub(crate) disk_image_location: String,
    pub(crate) kvm: QemuVmConfigKvm,
    pub(crate) uefi: Option<QemuVmConfigUefi>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QemuVmConfigExecutables {
    pub(crate) qemu: String,
    pub(crate) qemu_img: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QemuVmConfigKvm {
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QemuVmConfigUefi {
    pub(crate) code_file: String,
    pub(crate) vars_file: String,
}
