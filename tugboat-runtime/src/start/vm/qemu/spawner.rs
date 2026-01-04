use crate::start::config::VmConfig;
use crate::start::vm::qemu::{QemuVm, SizeInBytes};
use crate::start::vm::{RunVm, Spawner};
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
