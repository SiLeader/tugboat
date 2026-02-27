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
use tokio::fs::copy;
use tokio::process::Command;
use tracing::{debug, info};

pub struct BootDisk(pub String);

impl QemuVm<'_> {
    pub async fn create_boot_disk(&self) -> crate::Result<BootDisk> {
        info!("Creating boot disk");
        let path = self.args.image.as_str();
        let disk = format!("{}/{}.qcow2", self.config.disk_image_location, self.args.id);
        // debug!("Calling qemu-img create -f qcow2 -b {path} -F qcow2 {disk}");
        // let mut child = Command::new(&self.config.executables.qemu_img)
        //     .args(["create", "-f", "qcow2", "-b", path, "-F", "qcow2", &disk])
        //     .spawn()?;
        // child.wait().await?;
        copy(path, &disk).await?;
        debug!("qemu-img finished");
        Ok(BootDisk(disk))
    }
}
