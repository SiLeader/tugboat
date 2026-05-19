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

use crate::CloudHypervisorVmConfig;
use crate::cmd::api::ChApiClient;
use crate::execute::vm::RunVm;
use crate::validate::validate_ch_option_value;
use async_trait::async_trait;
use serde_json::json;
pub use spawner::CloudHypervisorVmBuilder;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Duration;
use tracing::{debug, info, warn};
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::run::{VmRunRequest, VmVolumeKind};
use volume_copy::BootDisk;

#[derive(Debug, Clone)]
struct CloudHypervisorVm<'a> {
    config: &'a CloudHypervisorVmConfig,
    args: VmRunRequest,
}

impl<'a> CloudHypervisorVm<'a> {
    fn new(config: &'a CloudHypervisorVmConfig, args: VmRunRequest) -> Self {
        Self { config, args }
    }

    fn validate_ch_inputs(&self, boot_disk: &BootDisk) -> crate::Result<()> {
        validate_ch_option_value(&boot_disk.0, "boot disk path")?;
        for (idx, network) in self.args.networks.iter().enumerate() {
            validate_ch_option_value(&network.iface_name, &format!("networks[{idx}].iface_name"))?;
            validate_ch_option_value(
                &network.mac_address,
                &format!("networks[{idx}].mac_address"),
            )?;
        }
        for (idx, volume) in self.args.volumes.iter().enumerate() {
            match volume.kind {
                VmVolumeKind::Block => {
                    validate_ch_option_value(
                        &volume.host_path,
                        &format!("volumes[{idx}].host_path"),
                    )?;
                }
                VmVolumeKind::Filesystem => {
                    return Err(crate::Error::ActionFailed(format!(
                        "filesystem volumes are not supported for Cloud Hypervisor until a virtio-fs backend is configured (volumes[{idx}])"
                    )));
                }
            }
        }
        Ok(())
    }

    fn boot_args(&self) -> crate::Result<Vec<String>> {
        if self.args.uefi.enabled {
            let firmware = self.config.boot.firmware.as_ref().ok_or_else(|| {
                crate::Error::Validation(
                    "UEFI boot requested but cloud_hypervisor.boot.firmware is not configured"
                        .into(),
                )
            })?;
            Ok(vec!["--firmware".into(), firmware.clone()])
        } else {
            let kernel = self.config.boot.kernel.as_ref().ok_or_else(|| {
                crate::Error::Validation(
                    "direct kernel boot requested but cloud_hypervisor.boot.kernel is not configured"
                        .into(),
                )
            })?;
            let mut args = vec!["--kernel".into(), kernel.clone()];
            if let Some(initramfs) = &self.config.boot.initramfs {
                args.push("--initramfs".into());
                args.push(initramfs.clone());
            }
            Ok(args)
        }
    }

    fn build_cli_args(&self, boot_disk: &BootDisk) -> crate::Result<Vec<String>> {
        validate_safe_id(&self.args.id, "vm id")?;
        self.validate_ch_inputs(boot_disk)?;

        let mut args = vec![
            "--api-socket".into(),
            self.config.get_api_socket_path(&self.args.id),
            "--event-monitor".into(),
            format!("path={}", self.config.get_event_path(&self.args.id)),
            "--cpus".into(),
            format!("boot={}", self.args.cpu.cores),
            "--memory".into(),
            format!("size={}", self.args.memory.size),
        ];

        if let Some(restore_handle) = &self.args.restore_handle {
            let source_id = self
                .args
                .restore_source_id
                .as_deref()
                .unwrap_or(&self.args.id);
            let ship_dir = tugboat_runtime_common::snapshot::ship_snapshot_dir(
                &self.config.snapshot_dir_path(),
                source_id,
            )
            .map_err(|e| crate::Error::Validation(e.to_string()))?;
            let snapshot_dir = ship_dir.join(restore_handle);
            if !snapshot_dir.exists() {
                return Err(crate::Error::Validation(format!(
                    "Restore requested but snapshot directory not found: {}",
                    snapshot_dir.display()
                )));
            }
            args.push("--restore".into());
            args.push(snapshot_dir.to_string_lossy().into_owned());
        } else {
            args.extend(self.boot_args()?);
        }

        args.push("--disk".into());
        args.push(format!("path={}", boot_disk.0));

        for network in &self.args.networks {
            args.push("--net".into());
            args.push(format!(
                "tap={},mac={}",
                network.iface_name, network.mac_address
            ));
        }

        for (idx, volume) in self.args.volumes.iter().enumerate() {
            match volume.kind {
                VmVolumeKind::Block => {
                    let readonly = if volume.read_only { "on" } else { "off" };
                    args.push("--disk".into());
                    args.push(format!("path={},readonly={readonly}", volume.host_path));
                }
                VmVolumeKind::Filesystem => {
                    return Err(crate::Error::ActionFailed(format!(
                        "filesystem volumes are not supported for Cloud Hypervisor until a virtio-fs backend is configured (volumes[{idx}])"
                    )));
                }
            }
        }

        args.extend([
            "--serial".into(),
            "tty".into(),
            "--console".into(),
            "off".into(),
        ]);

        Ok(args)
    }
}

#[async_trait]
impl RunVm for CloudHypervisorVm<'_> {
    async fn run_vm(&self) -> crate::Result<()> {
        info!("Starting Cloud Hypervisor VM");
        debug!("CloudHypervisorVm = {self:?}");

        let boot_disk = self.create_boot_disk().await?;
        let args = self.build_cli_args(&boot_disk)?;

        if let Some(incoming) = &self.args.incoming {
            self.run_for_incoming_migration(&args, incoming.port).await
        } else if self.args.restore_handle.is_some() {
            self.run_for_restore(&args).await
        } else {
            let err = Command::new(&self.config.executable)
                .args(&args)
                .debug_command()
                .exec();
            Err(crate::Error::Io(err))
        }
    }
}

impl CloudHypervisorVm<'_> {
    async fn run_for_restore(&self, args: &[String]) -> crate::Result<()> {
        info!("Starting Cloud Hypervisor to restore from snapshot");

        let mut child = tokio::process::Command::new(&self.config.executable)
            .args(args)
            .spawn()?;

        let socket_path = self.config.get_api_socket_path(&self.args.id);
        let mut client =
            match wait_for_api_socket(&socket_path, &mut child, Duration::from_secs(30)).await {
                Ok(client) => client,
                Err(e) => {
                    warn!("API socket wait failed, killing cloud-hypervisor: {e}");
                    let _ = child.kill().await;
                    return Err(e);
                }
            };

        info!("Resuming restored VM");
        if let Err(e) = client.put("/api/v1/vm.resume", None).await {
            warn!("vm.resume call failed, killing cloud-hypervisor: {e}");
            let _ = child.kill().await;
            return Err(e);
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err(crate::Error::Api(format!(
                "cloud-hypervisor exited with {status} after restore completed"
            )));
        }
        Ok(())
    }

    async fn run_for_incoming_migration(&self, args: &[String], port: u16) -> crate::Result<()> {
        info!("Starting Cloud Hypervisor to receive live migration on port {port}");

        let mut child = tokio::process::Command::new(&self.config.executable)
            .args(args)
            .spawn()?;

        let socket_path = self.config.get_api_socket_path(&self.args.id);
        let mut client =
            match wait_for_api_socket(&socket_path, &mut child, Duration::from_secs(30)).await {
                Ok(client) => client,
                Err(e) => {
                    warn!("API socket wait failed, killing cloud-hypervisor: {e}");
                    let _ = child.kill().await;
                    return Err(e);
                }
            };

        let receiver_url = format!("tcp:0.0.0.0:{port}");
        info!("Arming migration receiver with URL '{receiver_url}'");
        if let Err(e) = client
            .put(
                "/api/v1/vm.receive-migration",
                Some(&json!({ "receiver_url": receiver_url })),
            )
            .await
        {
            warn!("vm.receive-migration call failed, killing cloud-hypervisor: {e}");
            let _ = child.kill().await;
            return Err(e);
        }

        info!("Cloud Hypervisor is ready to receive live migration on port {port}");
        let status = child.wait().await?;
        if !status.success() {
            return Err(crate::Error::Api(format!(
                "cloud-hypervisor exited with {status} after incoming migration completed"
            )));
        }
        Ok(())
    }
}

async fn wait_for_api_socket(
    socket_path: &str,
    child: &mut tokio::process::Child,
    timeout: Duration,
) -> crate::Result<ChApiClient> {
    const POLL_INTERVAL: Duration = Duration::from_millis(200);
    let deadline = std::time::Instant::now() + timeout;

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(crate::Error::Api(format!(
                "timed out after {timeout:.0?} waiting for Cloud Hypervisor API socket '{socket_path}'"
            )));
        }

        if let Some(status) = child.try_wait()? {
            return Err(crate::Error::Api(format!(
                "cloud-hypervisor process exited prematurely with {status} \
                 before API socket '{socket_path}' became available"
            )));
        }

        match ChApiClient::connect(socket_path).await {
            Ok(client) => return Ok(client),
            Err(_) => tokio::time::sleep(POLL_INTERVAL).await,
        }
    }
}

trait DebugCommand: Sized {
    fn debug_command(&mut self) -> &mut Self;
}

impl DebugCommand for Command {
    fn debug_command(&mut self) -> &mut Self {
        debug!("Cloud Hypervisor: {:?}", self.get_program());
        let args = self
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        debug!("Args: {args}");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CloudHypervisorVm;
    use super::volume_copy::BootDisk;
    use crate::{CloudHypervisorBootConfig, CloudHypervisorVmConfig};
    use tugboat_vm_runtime_interface::run::{
        VmCpuConfig, VmDiskImageFormat, VmMemoryConfig, VmNetworkConfig, VmRunRequest,
        VmUefiConfig, VmVolumeConfig,
    };

    fn vm_config() -> CloudHypervisorVmConfig {
        CloudHypervisorVmConfig {
            executable: "/usr/bin/cloud-hypervisor".into(),
            disk_image_location: "/var/lib/tugboat".into(),
            boot: CloudHypervisorBootConfig {
                kernel: Some("/var/lib/tugboat/vmlinux".into()),
                initramfs: Some("/var/lib/tugboat/initramfs.img".into()),
                firmware: Some("/usr/share/ovmf.fd".into()),
            },
            snapshot_dir: None,
        }
    }

    fn run_request(uefi_enabled: bool) -> VmRunRequest {
        VmRunRequest {
            image: "/images/base.raw".into(),
            image_format: VmDiskImageFormat::Raw,
            cpu: VmCpuConfig {
                architecture: "x86_64".into(),
                cores: 4,
            },
            memory: VmMemoryConfig {
                size: 2 * 1024 * 1024,
            },
            id: "vm-01".into(),
            networks: vec![VmNetworkConfig {
                iface_name: "tap0".into(),
                mac_address: "02:00:00:00:00:01".into(),
            }],
            volumes: vec![VmVolumeConfig::block("/data/disk1.raw", "raw", true)],
            uefi: VmUefiConfig {
                enabled: uefi_enabled,
            },
            incoming: None,
            restore_handle: None,
            restore_source_id: None,
            user: Default::default(),
        }
    }

    #[test]
    fn builds_cli_args_for_direct_kernel_boot() {
        let config = vm_config();
        let vm = CloudHypervisorVm::new(&config, run_request(false));
        let args = vm
            .build_cli_args(&BootDisk("/var/lib/tugboat/vm-01.img".into()))
            .unwrap();

        assert_eq!(
            args,
            vec![
                "--api-socket",
                "/var/lib/tugboat/vm-01.ch.sock",
                "--event-monitor",
                "path=/var/lib/tugboat/vm-01.events",
                "--cpus",
                "boot=4",
                "--memory",
                "size=2097152",
                "--kernel",
                "/var/lib/tugboat/vmlinux",
                "--initramfs",
                "/var/lib/tugboat/initramfs.img",
                "--disk",
                "path=/var/lib/tugboat/vm-01.img",
                "--net",
                "tap=tap0,mac=02:00:00:00:00:01",
                "--disk",
                "path=/data/disk1.raw,readonly=on",
                "--serial",
                "tty",
                "--console",
                "off",
            ]
        );
    }

    #[test]
    fn builds_cli_args_for_uefi_boot() {
        let config = vm_config();
        let vm = CloudHypervisorVm::new(&config, run_request(true));
        let args = vm
            .build_cli_args(&BootDisk("/var/lib/tugboat/vm-01.img".into()))
            .unwrap();

        assert!(args.windows(2).any(|window| {
            window == ["--firmware".to_string(), "/usr/share/ovmf.fd".to_string()]
        }));
        assert!(!args.iter().any(|arg| arg == "--kernel"));
        assert!(!args.iter().any(|arg| arg == "--initramfs"));
    }

    #[test]
    fn rejects_cloud_hypervisor_delimiters_in_option_values() {
        let mut request = run_request(false);
        request.networks[0].iface_name = "tap,0".into();
        let config = vm_config();
        let vm = CloudHypervisorVm::new(&config, request);

        let err = vm
            .build_cli_args(&BootDisk("/var/lib/tugboat/vm-01.img".into()))
            .unwrap_err();

        assert!(matches!(err, crate::Error::Validation(_)));
    }

    #[test]
    fn rejects_filesystem_volumes_until_virtiofs_backend_exists() {
        let config = vm_config();
        let mut request = run_request(false);
        request
            .volumes
            .push(VmVolumeConfig::filesystem("/shared/fs", "shared", false));
        let vm = CloudHypervisorVm::new(&config, request);

        let err = vm
            .build_cli_args(&BootDisk("/var/lib/tugboat/vm-01.img".into()))
            .unwrap_err();

        match err {
            crate::Error::ActionFailed(message) => {
                assert!(message.contains("filesystem volumes are not supported"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
