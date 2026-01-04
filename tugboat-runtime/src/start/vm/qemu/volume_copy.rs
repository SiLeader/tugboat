use crate::start::vm::qemu::QemuVm;
use tokio::process::Command;

pub(crate) struct BootDisk(pub String);

impl QemuVm<'_> {
    async fn fetch_image(&self) -> crate::Result<String> {
        Ok(self.image.clone())
    }

    pub(crate) async fn create_boot_disk(&self) -> crate::Result<BootDisk> {
        let path = self.fetch_image().await?;
        let disk = format!("{}/{}.qcow2", self.config.disk_image_location, self.id);
        let mut child = Command::new(&self.config.executables.qemu_img)
            .args(["create", "-f", "qcow2", "-b", &path, "-F", "qcow2", &disk])
            .spawn()?;
        child.wait().await?;
        Ok(BootDisk(disk))
    }
}
