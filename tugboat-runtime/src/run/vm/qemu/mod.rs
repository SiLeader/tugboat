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

mod spawner;
mod volume_copy;

use crate::run::vm::RunVm;
use crate::run::vm::qemu::spawner::QemuVmConfigUefi;
use crate::run::vm::qemu::volume_copy::BootDisk;
use async_trait::async_trait;
pub use spawner::{QemuVmBuilder, QemuVmConfig};
use std::fs::copy;
use std::os::unix::process::CommandExt;
use std::process::Command;
use tracing::debug;
use tugboat_resources::manifests::core::v1::CpuSpec;

#[derive(Debug, Clone)]
struct QemuVm<'a> {
    config: &'a QemuVmConfig,
    image: String,
    cpu: CpuSpec,
    memory: SizeInBytes,
    id: String,
}

#[derive(Debug, Clone)]
struct SizeInBytes(u64);

impl<'a> QemuVm<'a> {
    fn new(
        config: &'a QemuVmConfig,
        image: String,
        cpu: CpuSpec,
        memory: SizeInBytes,
        id: String,
    ) -> Self {
        Self {
            config,
            image,
            cpu,
            memory,
            id,
        }
    }
}

#[async_trait]
impl RunVm for QemuVm<'_> {
    async fn run_vm(&self) -> crate::Result<()> {
        let img = self.create_boot_disk().await?;
        let qmp_uds = format!(
            "unix:{}/{}.qmp.sock",
            self.config.disk_image_location, self.id
        );
        let err = Command::new(&self.config.executables.qemu)
            .args(["-machine", "q35"])
            .args(["-nographic"])
            .args(["-net", "none"]) // TODO
            .args(["-qmp", qmp_uds.as_str()])
            .args_if(self.config.kvm.enabled, &["-enable-kvm"])
            .qemu_args(&self.cpu)
            .qemu_args(&self.memory)
            .qemu_args(&img)
            .qemu_args_with_arg(&self.config.uefi, &self)
            .debug_command()
            .exec();
        panic!("Cannot exec: {err}");
    }
}

trait QemuArgs<T>: Sized {
    fn qemu_args(&mut self, value: &T) -> &mut Self;
}

trait QemuArgsWithArg<T, A>: Sized {
    fn qemu_args_with_arg(&mut self, value: &T, arg: &A) -> &mut Self;
}

trait DebugCommand: Sized {
    fn debug_command(&mut self) -> &mut Self;
}

trait ConditionalArgs: Sized {
    fn args_if(&mut self, predicate: bool, args: &[&str]) -> &mut Self;
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

impl QemuArgs<SizeInBytes> for Command {
    fn qemu_args(&mut self, value: &SizeInBytes) -> &mut Self {
        let megs = value.0 / 1024 / 1024;
        self.args(["-m", megs.to_string().as_str()])
    }
}

impl QemuArgs<BootDisk> for Command {
    fn qemu_args(&mut self, value: &BootDisk) -> &mut Self {
        let opts = format!("if=virtio,format=qcow2,index=0,media=disk,file={}", value.0);
        self.arg("-drive").arg(opts)
    }
}

impl QemuArgsWithArg<Option<QemuVmConfigUefi>, QemuVm<'_>> for Command {
    fn qemu_args_with_arg(
        &mut self,
        value: &Option<QemuVmConfigUefi>,
        this: &QemuVm<'_>,
    ) -> &mut Self {
        if let Some(uefi) = value {
            let code_opts = format!("if=pflash,format=raw,readonly=on,file={}", uefi.code_file);
            let vars_location =
                format!("{}/{}.uefi.vars", this.config.disk_image_location, this.id);
            copy(&uefi.vars_file, &vars_location).expect("Cannot copy vars file"); // TODO
            let vars_opts = format!("if=pflash,format=raw,file={}", vars_location);
            self.args(["-drive", &code_opts, "-drive", &vars_opts])
        } else {
            self
        }
    }
}

impl ConditionalArgs for Command {
    fn args_if(&mut self, predicate: bool, args: &[&str]) -> &mut Self {
        if predicate { self.args(args) } else { self }
    }
}

impl DebugCommand for Command {
    fn debug_command(&mut self) -> &mut Self {
        debug!("QEMU: {:?}", self.get_program());
        let args = self
            .get_args()
            .map(|c| c.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        debug!("Args: {args}");
        self
    }
}
