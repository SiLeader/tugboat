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
use std::path::PathBuf;
use tokio::fs::{copy, metadata};
use tracing::{debug, info};
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::run::VmDiskImageFormat;

#[derive(Debug, Clone)]
pub struct BootDisk {
    pub path: String,
    pub format: VmDiskImageFormat,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootDiskState {
    pub path: String,
    pub format: VmDiskImageFormat,
}

pub(crate) fn boot_disk_extension(format: VmDiskImageFormat) -> &'static str {
    match format {
        VmDiskImageFormat::Qcow2 => "qcow2",
        VmDiskImageFormat::Raw => "raw",
    }
}

pub(crate) fn boot_disk_format_name(format: VmDiskImageFormat) -> &'static str {
    match format {
        VmDiskImageFormat::Qcow2 => "qcow2",
        VmDiskImageFormat::Raw => "raw",
    }
}

impl QemuVm<'_> {
    pub(crate) fn boot_disk_path(&self, id: &str, format: VmDiskImageFormat) -> PathBuf {
        PathBuf::from(&self.config.disk_image_location)
            .join(format!("{id}.{}", boot_disk_extension(format)))
    }

    pub(crate) fn boot_disk_state_path(&self, id: &str) -> PathBuf {
        PathBuf::from(&self.config.disk_image_location).join(format!("{id}.boot.json"))
    }

    async fn write_boot_disk_state(&self, disk: &std::path::Path) -> crate::Result<()> {
        let state = BootDiskState {
            path: disk.to_string_lossy().into_owned(),
            format: self.args.image_format,
        };
        let state_path = self.boot_disk_state_path(&self.args.id);
        let contents = serde_json::to_vec(&state)?;
        tokio::fs::write(state_path, contents).await?;
        Ok(())
    }

    pub async fn create_boot_disk(&self) -> crate::Result<BootDisk> {
        info!("Creating boot disk");
        let disk = self.boot_disk_path(&self.args.id, self.args.image_format);
        if self.args.restore_handle.is_some() {
            let source_id = self
                .args
                .restore_source_id
                .as_deref()
                .unwrap_or(&self.args.id);
            validate_safe_id(source_id, "restore source vm id")?;
            let source_disk = self.boot_disk_path(source_id, self.args.image_format);
            if source_disk == disk {
                metadata(&disk).await?;
            } else {
                copy(&source_disk, &disk).await?;
            }
        } else {
            let path = self.args.image.as_str();
            // debug!("Calling qemu-img create -f qcow2 -b {path} -F qcow2 {disk:?}");
            // let mut child = Command::new(&self.config.executables.qemu_img)
            //     .args(["create", "-f", "qcow2", "-b", path, "-F", "qcow2", disk.as_os_str()])
            //     .spawn()?;
            // child.wait().await?;
            copy(path, &disk).await?;
        }
        self.write_boot_disk_state(&disk).await?;
        debug!("Boot disk prepared");
        Ok(BootDisk {
            path: disk.to_string_lossy().into_owned(),
            format: self.args.image_format,
        })
    }
}
