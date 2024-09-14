mod error;
mod volume_copy;

use crate::vm::RunVm;
use async_trait::async_trait;
use resources::manifests::core::v1::{
    CpuSpec, MachineSpec, MemorySpec, PersistentVolumeSpec, VolumeOptions,
};
use resources::manifests::ObjectMeta;
use tokio::process::{Child, Command};

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
    async fn run_vm(&self) -> crate::Result<Child> {
        self.copy_image_to_boot_disk().await?;
        let child = Command::new(self.executable.as_str())
            .args(["-enable-kvm", "-machine", "q35"])
            .qemu_args(&self.machine.cpu)
            .qemu_args(&self.machine.memory)
            .qemu_args_with_arg(&self.boot_disk_volume, 0)
            .qemu_args(&self.persistent_volumes)
            .spawn()
            .map_err(crate::Error::Io)?;
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

impl QemuArgsWithArg<PersistentVolumeSpec, usize> for Command {
    fn qemu_args_with_arg(&mut self, value: &PersistentVolumeSpec, index: usize) -> &mut Self {
        match &value.options {
            VolumeOptions::Nbd { nbd } => {
                let arg = format!("file=nbd:{},index={index}", nbd.server);
                self.args(["-drive", arg.as_str()])
            }
            VolumeOptions::Local { local } => {
                let file = format!(
                    "format=raw,file={},cache=writethrough,index={index}",
                    local.file
                );
                self.args(["-drive", file.as_str()])
            }
        }
    }
}

impl QemuArgs<Vec<PersistentVolumeSpec>> for Command {
    fn qemu_args(&mut self, value: &Vec<PersistentVolumeSpec>) -> &mut Self {
        let mut this = self;
        for (index, pv) in value.iter().enumerate() {
            this = this.qemu_args_with_arg(pv, index + 1);
        }
        this
    }
}
