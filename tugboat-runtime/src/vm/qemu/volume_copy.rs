use crate::vm::qemu::QemuVm;
use resources::manifests::core::v1::VolumeOptions;
use tokio::process::Command;

impl QemuVm {
    pub(super) async fn copy_image_to_boot_disk(&self) -> crate::Result<()> {
        let mut proc = match &self.boot_disk_volume.options {
            VolumeOptions::Nbd { .. } => {
                todo!()
            }
            VolumeOptions::Local { local } => Command::new("qemu-img")
                .args(["convert", "-f", "qcow2", "-O", "raw", local.file.as_str()])
                .spawn()
                .map_err(crate::Error::Io)?,
        };

        let status = proc.wait().await.map_err(crate::Error::Io)?;

        if status.success() {
            Ok(())
        } else {
            Err(crate::Error::Copy)
        }
    }
}
