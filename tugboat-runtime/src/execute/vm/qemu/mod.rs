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

use crate::execute::vm::RunVm;
use crate::execute::vm::qemu::spawner::QemuVmConfigUefi;
use crate::execute::vm::qemu::volume_copy::BootDisk;
use async_trait::async_trait;
pub use spawner::{QemuVmBuilder, QemuVmConfig};
use std::os::unix::process::CommandExt;
use std::process::Command;
use tracing::{debug, info};
use tugboat_vm_runtime_interface::run::{
    VmCpuConfig, VmIncomingMigrationConfig, VmMemoryConfig, VmNetworkConfig, VmRunRequest,
    VmVolumeConfig, VmVolumeKind,
};

#[derive(Debug, Clone)]
struct QemuVm<'a> {
    config: &'a QemuVmConfig,
    args: VmRunRequest,
}

impl<'a> QemuVm<'a> {
    fn new(config: &'a QemuVmConfig, args: VmRunRequest) -> Self {
        Self { config, args }
    }

    async fn prepare_uefi(&self) -> crate::Result<()> {
        if let Some(uefi_config) = &self.config.uefi
            && self.args.uefi.enabled
        {
            let vars_location = format!(
                "{}/{}.uefi.vars",
                self.config.disk_image_location, self.args.id
            );
            debug!(
                "Copying UEFI vars from '{}' to '{}'",
                uefi_config.vars_file, vars_location
            );
            tokio::fs::copy(&uefi_config.vars_file, &vars_location).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl RunVm for QemuVm<'_> {
    async fn run_vm(&self) -> crate::Result<()> {
        info!("Starting QEMU VM");
        debug!("QemuVm = {self:?}");
        let img = self.create_boot_disk().await?;
        self.prepare_uefi().await?;

        let qmp_uds = self.config.get_uds_url(&self.args.id);
        let qmp_opt = format!("{qmp_uds},server=on,wait=off");
        debug!("QEMU UDS = {qmp_uds}");
        let err = Command::new(&self.config.executables.qemu)
            .args(["-machine", "q35"])
            .args(["-nographic"])
            .args(["-qmp", qmp_opt.as_str()])
            .args_if(self.config.kvm.enabled, &["-enable-kvm"])
            .qemu_args(&self.args.cpu)
            .qemu_args(&self.args.memory)
            .qemu_args(&self.args.networks)
            .qemu_args(&self.args.volumes)
            .qemu_args(&img)
            .qemu_args(&self.args.incoming)
            .qemu_args_with_arg_if(self.args.uefi.enabled, &self.config.uefi, &self)
            .debug_command()
            .exec();
        panic!("Cannot exec: {err}");
    }
}

trait QemuArgs<T>: Sized {
    fn qemu_args(&mut self, value: &T) -> &mut Self;
}

trait QemuArgsWithArgIf<T, A>: Sized {
    fn qemu_args_with_arg_if(&mut self, predicate: bool, value: &T, arg: &A) -> &mut Self;
}

trait DebugCommand: Sized {
    fn debug_command(&mut self) -> &mut Self;
}

trait ConditionalArgs: Sized {
    fn args_if(&mut self, predicate: bool, args: &[&str]) -> &mut Self;
}

impl QemuArgs<VmCpuConfig> for Command {
    fn qemu_args(&mut self, value: &VmCpuConfig) -> &mut Self {
        let smp = value.cores;
        if smp > 0 {
            let smp_arg = format!("{smp},cores={}", value.cores);
            self.args(["-smp", smp_arg.as_str()])
        } else {
            self
        }
    }
}

impl QemuArgs<VmMemoryConfig> for Command {
    fn qemu_args(&mut self, value: &VmMemoryConfig) -> &mut Self {
        let megs = (value.size / 1024 / 1024).max(1);
        let memory_arg = format!("{megs}M");
        self.args(["-m", memory_arg.as_str()])
    }
}

impl QemuArgs<BootDisk> for Command {
    fn qemu_args(&mut self, value: &BootDisk) -> &mut Self {
        let opts = format!("if=virtio,format=qcow2,index=0,media=disk,file={}", value.0);
        self.arg("-drive").arg(opts)
    }
}

impl QemuArgs<Vec<VmNetworkConfig>> for Command {
    fn qemu_args(&mut self, value: &Vec<VmNetworkConfig>) -> &mut Self {
        for (idx, network) in value.iter().enumerate() {
            let opts = format!(
                "tap,id=net{idx},ifname={},script=no,downscript=no",
                network.iface_name
            );
            self.arg("-netdev").arg(opts);

            let dev = format!("virtio-net-pci,netdev=net{idx},mac={}", network.mac_address);
            self.arg("-device").arg(dev);
        }
        self
    }
}

impl QemuArgs<Vec<VmVolumeConfig>> for Command {
    fn qemu_args(&mut self, value: &Vec<VmVolumeConfig>) -> &mut Self {
        let mut disk_index = 1;
        for (idx, vol) in value.iter().enumerate() {
            match vol.kind {
                VmVolumeKind::Block => {
                    let ro = if vol.read_only { "on" } else { "off" };
                    let opts = format!(
                        "if=virtio,format={},index={},media=disk,readonly={},file={}",
                        vol.format, disk_index, ro, vol.host_path
                    );
                    self.arg("-drive").arg(opts);
                    disk_index += 1;
                }
                VmVolumeKind::Filesystem => {
                    let fsdev_id = format!("fs{}", idx + 1);
                    let mut fsdev_opts = format!(
                        "local,id={fsdev_id},path={},security_model=none",
                        vol.host_path
                    );
                    if vol.read_only {
                        fsdev_opts.push_str(",readonly=on");
                    }
                    let device_opts =
                        format!("virtio-9p-pci,fsdev={fsdev_id},mount_tag={}", vol.mount_tag);
                    self.arg("-fsdev").arg(fsdev_opts);
                    self.arg("-device").arg(device_opts);
                }
            }
        }
        self
    }
}

impl QemuArgs<Option<VmIncomingMigrationConfig>> for Command {
    fn qemu_args(&mut self, value: &Option<VmIncomingMigrationConfig>) -> &mut Self {
        if let Some(config) = value {
            let incoming = format!("tcp:0.0.0.0:{}", config.port);
            self.args(["-incoming", incoming.as_str()])
        } else {
            self
        }
    }
}

impl QemuArgsWithArgIf<Option<QemuVmConfigUefi>, QemuVm<'_>> for Command {
    fn qemu_args_with_arg_if(
        &mut self,
        predicate: bool,
        value: &Option<QemuVmConfigUefi>,
        this: &QemuVm<'_>,
    ) -> &mut Self {
        if let Some(uefi) = value
            && predicate
        {
            let code_opts = format!("if=pflash,format=raw,readonly=on,file={}", uefi.code_file);
            let vars_location = format!(
                "{}/{}.uefi.vars",
                this.config.disk_image_location, this.args.id
            );
            // vars_location should have been prepared by prepare_uefi
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

#[cfg(test)]
mod tests {
    use super::QemuArgs;
    use std::process::Command;
    use tugboat_vm_runtime_interface::run::VmVolumeConfig;

    #[test]
    fn block_volume_path_is_forwarded_to_drive_args() {
        let mut command = Command::new("qemu-system-x86_64");
        command.qemu_args(&vec![VmVolumeConfig::block(
            "/var/lib/tugboat-agent/csi/ship-uid/data-disk.block",
            "raw",
            true,
        )]);

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "-drive".to_string(),
                "if=virtio,format=raw,index=1,media=disk,readonly=on,file=/var/lib/tugboat-agent/csi/ship-uid/data-disk.block".to_string(),
            ]
        );
    }

    #[test]
    fn filesystem_volume_path_is_forwarded_to_9p_args() {
        let mut command = Command::new("qemu-system-x86_64");
        command.qemu_args(&vec![VmVolumeConfig::filesystem(
            "/var/lib/tugboat-agent/csi/ship-uid/data-disk.fs",
            "data-disk",
            false,
        )]);

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "-fsdev".to_string(),
                "local,id=fs1,path=/var/lib/tugboat-agent/csi/ship-uid/data-disk.fs,security_model=none".to_string(),
                "-device".to_string(),
                "virtio-9p-pci,fsdev=fs1,mount_tag=data-disk".to_string(),
            ]
        );
    }
}
