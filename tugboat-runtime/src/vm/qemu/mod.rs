mod error;
mod volume_copy;

use crate::vm::RunVm;
use crate::vm::qemu::volume_copy::BootDisk;
use async_trait::async_trait;
use resources::manifests::ObjectMeta;
use resources::manifests::core::v1::{CpuSpec, MachineSpec, MemorySpec};
use tokio::process::{Child, Command};

#[derive(Debug, Clone)]
struct QemuVm {
    qemu: String,
    qemu_img: String,
    disk_image_location: String,
    ship_ref: ObjectMeta,
    image: String,
    machine: MachineSpec,
    id: String,
}

#[async_trait]
impl RunVm for QemuVm {
    async fn run_vm(&self) -> crate::Result<Child> {
        let img = self.create_boot_disk().await?;
        let child = Command::new(&self.qemu)
            .args(["-enable-kvm", "-machine", "q35"])
            .qemu_args(&self.machine.cpu)
            .qemu_args(&self.machine.memory)
            .qemu_args(&img)
            .spawn()?;
        Ok(child)
    }
}

trait QemuArgs<T>: Sized {
    fn qemu_args(&mut self, value: &T) -> &mut Self;
}

trait QemuArgsWithArg<T, A>: Sized {
    fn qemu_args_with_arg(&mut self, value: &T, arg: A) -> &mut Self;
}

impl QemuArgs<CpuSpec> for Command {
    fn qemu_args(&mut self, value: &CpuSpec) -> &mut Self {
        let smp = value.threads_per_core * value.cores * value.dies * value.sockets;
        if smp > 0 {
            let smp_arg = format!(
                "{smp},sockets={},dies={},cores={},threads={}",
                value.sockets, value.dies, value.cores, value.threads_per_core
            );
            self.args(["-smp", smp_arg.as_str()])
        } else {
            self
        }
    }
}

impl QemuArgs<MemorySpec> for Command {
    fn qemu_args(&mut self, value: &MemorySpec) -> &mut Self {
        let megs = value.0.as_byte_length().unwrap_or(1024 * 1024) / 1024;
        self.args(["-m", megs.to_string().as_str()])
    }
}

impl QemuArgs<BootDisk> for Command {
    fn qemu_args(&mut self, value: &BootDisk) -> &mut Self {
        let opts = format!("file={},format=qcow2,if=virtio", value.0);
        self.arg("-drive").arg(opts)
    }
}
