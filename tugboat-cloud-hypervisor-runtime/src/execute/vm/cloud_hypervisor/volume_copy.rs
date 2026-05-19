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

use crate::execute::vm::cloud_hypervisor::CloudHypervisorVm;
use std::path::PathBuf;
use tokio::fs::copy;
use tracing::info;
use tugboat_vm_runtime_interface::run::VmDiskImageFormat;

#[derive(Debug, Clone)]
pub struct BootDisk {
    pub path: String,
    pub format: VmDiskImageFormat,
}

pub(crate) fn boot_disk_extension(format: VmDiskImageFormat) -> &'static str {
    match format {
        VmDiskImageFormat::Raw => "raw",
        VmDiskImageFormat::Qcow2 => "qcow2",
    }
}

impl CloudHypervisorVm<'_> {
    fn boot_disk_path(&self) -> PathBuf {
        PathBuf::from(&self.config.disk_image_location).join(format!(
            "{}.{}",
            self.args.id,
            boot_disk_extension(self.args.image_format)
        ))
    }

    pub async fn create_boot_disk(&self) -> crate::Result<BootDisk> {
        if self.args.image_format != VmDiskImageFormat::Raw {
            return Err(crate::Error::Validation(
                "Cloud Hypervisor runtime supports raw boot images only; use imageFormat=raw"
                    .to_string(),
            ));
        }

        info!("Creating boot disk");
        let disk = self.boot_disk_path();
        copy(&self.args.image, &disk).await?;
        Ok(BootDisk {
            path: disk.to_string_lossy().into_owned(),
            format: self.args.image_format,
        })
    }
}
