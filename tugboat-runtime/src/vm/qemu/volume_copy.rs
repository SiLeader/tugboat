use crate::vm::qemu::QemuVm;
use resources::manifests::core::v1::VolumeOptions;
use tokio::process::Command;

impl QemuVm {
    pub(super) async fn copy_image_to_boot_disk(&self) -> std::io::Result<()> {
        let mut proc = match &self.boot_disk_volume.options {
            VolumeOptions::Nbd { .. } => {
                todo!()
            }
            VolumeOptions::Local { local } => Command::new("qemu-img")
                .args(["convert", "-f", "qcow2", "-O", "qcow2", local.file.as_str()])
                .spawn()?,
        };
        let status = proc.wait().await?;

        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::Other))
        }
    }
}
