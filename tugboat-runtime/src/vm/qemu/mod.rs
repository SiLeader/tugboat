mod error;
mod volume_copy;

use crate::vm::RunVm;
use async_trait::async_trait;
use resources::manifests::core::v1::{CpuSpec, MachineSpec, MemorySpec, PersistentVolumeSpec};
use resources::manifests::ObjectMeta;
use tokio::process::Command;

#[derive(Debug, Clone)]
struct QemuVm {
    executable: String,
    ship_ref: ObjectMeta,
    image: String,
    machine: MachineSpec,
    boot_disk_volume: PersistentVolumeSpec,
    persistent_volumes: Vec<PersistentVolumeSpec>,
}

#[async_trait]
impl RunVm for QemuVm {
    async fn run_vm(&self) {
        self.copy_image_to_boot_disk().await;
        let mut cmd = Command::new(self.executable.as_str())
            .args(["-machine", "q35,accel=kvm"])
            .qemu_args(&self.machine.cpu)
            .qemu_args(&self.machine.memory)
            .qemu_args(&self.boot_disk_volume);
        for pv in &self.persistent_volumes {
            cmd = cmd.qemu_args(pv);
        }
    }
}

trait QemuArgs<T>: Sized {
    fn qemu_args(&mut self, value: &T) -> &mut Self;
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

impl QemuArgs<PersistentVolumeSpec> for Command {
    fn qemu_args(&mut self, value: &PersistentVolumeSpec) -> &mut Self {
        self.args(value.qemu_args.as_slice())
    }
}
