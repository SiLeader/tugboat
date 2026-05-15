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

pub struct BootDisk(pub String);

impl QemuVm<'_> {
    fn boot_disk_path(&self, id: &str) -> PathBuf {
        PathBuf::from(&self.config.disk_image_location).join(format!("{id}.qcow2"))
    }

    pub async fn create_boot_disk(&self) -> crate::Result<BootDisk> {
        info!("Creating boot disk");
        let disk = self.boot_disk_path(&self.args.id);
        if self.args.restore_handle.is_some() {
            let source_id = self
                .args
                .restore_source_id
                .as_deref()
                .unwrap_or(&self.args.id);
            validate_safe_id(source_id, "restore source vm id")?;
            let source_disk = self.boot_disk_path(source_id);
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
        debug!("Boot disk prepared");
        Ok(BootDisk(disk.to_string_lossy().into_owned()))
    }
}
