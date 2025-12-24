use crate::RuntimeArgs;
use crate::vm::qemu::{QemuVm, SizeInBytes};
use crate::vm::{RunVm, Spawner};
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
    async fn spawn(&self, args: RuntimeArgs) -> crate::Result<()> {
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
    pub(super) executables: QemuVmConfigExecutables,
    pub(super) disk_image_location: String,
    pub(super) kvm: QemuVmConfigKvm,
    pub(super) uefi: Option<QemuVmConfigUefi>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct QemuVmConfigExecutables {
    pub(super) qemu: String,
    pub(super) qemu_img: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct QemuVmConfigKvm {
    pub(super) enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct QemuVmConfigUefi {
    pub(super) code_file: String,
    pub(super) vars_file: String,
}
