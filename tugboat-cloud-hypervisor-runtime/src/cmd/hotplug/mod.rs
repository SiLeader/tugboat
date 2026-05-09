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

use crate::CloudHypervisorVmConfig;
use crate::cmd::api::ChApiClient;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};
use tugboat_runtime_common::config::load_config;
use tugboat_vm_runtime_interface::hotplug::{
    VmHotplugRequest, normalize_identifier_key, sanitize_identifier,
    validate_and_normalize_hotplug_request,
};
use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig, VmVolumeKind};

#[derive(Debug, Parser)]
pub struct HotplugArgs {
    #[arg(help = "Path to the hotplug request config file or - for stdin")]
    config: String,
}

#[derive(Debug)]
enum HotplugRollback {
    ResizeCpu { previous_vcpus: u64 },
    ResizeMemory { previous_ram: u64 },
    RemoveNic { id: String },
    AddNic { config: HotplugNetConfig },
    RemoveDisk { id: String },
    AddDisk { config: HotplugDiskConfig },
}

#[derive(Debug, Clone, Default)]
struct VmRuntimeState {
    vcpus: u64,
    ram_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct VmInfoResponse {
    config: VmInfoConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct VmInfoConfig {
    cpus: VmInfoCpuConfig,
    memory: VmInfoMemoryConfig,
    #[serde(default)]
    net: Vec<VmInfoNetConfig>,
    #[serde(default)]
    disks: Vec<VmInfoDiskConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct VmInfoCpuConfig {
    #[serde(rename = "boot_vcpus", alias = "boot")]
    boot_vcpus: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct VmInfoMemoryConfig {
    size: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct VmInfoNetConfig {
    tap: Option<String>,
    mac: Option<String>,
    id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VmInfoDiskConfig {
    path: Option<String>,
    #[serde(default)]
    readonly: bool,
    id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HotplugNetConfig {
    tap: String,
    mac: String,
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HotplugDiskConfig {
    path: String,
    readonly: bool,
    id: String,
}

pub async fn run(config: CloudHypervisorVmConfig, args: HotplugArgs) -> crate::Result<()> {
    let req: VmHotplugRequest = load_config(args.config)?;
    hotplug_request(&config, &req).await
}

async fn hotplug_request(
    config: &CloudHypervisorVmConfig,
    req: &VmHotplugRequest,
) -> crate::Result<()> {
    let req = validate_and_normalize_hotplug_request(req.clone())
        .map_err(|err| crate::Error::Validation(err.to_string()))?;

    let socket_path = config.get_api_socket_path(&req.id);
    let mut client = ChApiClient::connect(&socket_path).await?;
    let vm_info = fetch_vm_info(&mut client).await?;
    let mut state = VmRuntimeState::from_vm_info(&vm_info);
    let mut rollback = Vec::new();

    let result =
        apply_hotplug_changes(&mut client, &req, &vm_info, &mut state, &mut rollback).await;
    if let Err(ref err) = result {
        warn!("hotplug failed ({err}); attempting best-effort rollback of applied changes");
        execute_rollback(&mut client, &mut state, rollback).await;
    }
    result
}

async fn fetch_vm_info(client: &mut ChApiClient) -> crate::Result<VmInfoResponse> {
    Ok(serde_json::from_value(
        client.get("/api/v1/vm.info").await?,
    )?)
}

async fn apply_hotplug_changes(
    client: &mut ChApiClient,
    req: &VmHotplugRequest,
    vm_info: &VmInfoResponse,
    state: &mut VmRuntimeState,
    rollback: &mut Vec<HotplugRollback>,
) -> crate::Result<()> {
    if let Some(cpu) = &req.cpu {
        apply_cpu_resize(client, state, cpu.cores, rollback).await?;
    }

    if let Some(memory) = &req.memory {
        apply_memory_resize(client, state, memory.size, rollback).await?;
    }

    for id in &req.nics_removed {
        let config = find_nic_config_for_rollback(vm_info, id)?;
        apply_nic_remove(client, id).await?;
        rollback.push(HotplugRollback::AddNic { config });
    }

    for nic in &req.nics_added {
        let config = HotplugNetConfig::from_network(nic);
        apply_nic_add(client, &config).await?;
        rollback.push(HotplugRollback::RemoveNic {
            id: config.id.clone(),
        });
    }

    for id in &req.volumes_removed {
        let config = find_disk_config_for_rollback(vm_info, id)?;
        apply_disk_remove(client, id).await?;
        rollback.push(HotplugRollback::AddDisk { config });
    }

    for volume in &req.volumes_added {
        let config = HotplugDiskConfig::from_volume(volume)?;
        apply_disk_add(client, &config).await?;
        rollback.push(HotplugRollback::RemoveDisk {
            id: config.id.clone(),
        });
    }

    Ok(())
}

async fn apply_cpu_resize(
    client: &mut ChApiClient,
    state: &mut VmRuntimeState,
    target_vcpus: u64,
    rollback: &mut Vec<HotplugRollback>,
) -> crate::Result<()> {
    if target_vcpus == state.vcpus {
        return Ok(());
    }

    info!("Resizing VM CPUs from {} to {}", state.vcpus, target_vcpus);
    let previous_vcpus = state.vcpus;
    resize_vm(client, target_vcpus, state.ram_bytes).await?;
    state.vcpus = target_vcpus;
    rollback.push(HotplugRollback::ResizeCpu { previous_vcpus });
    Ok(())
}

async fn apply_memory_resize(
    client: &mut ChApiClient,
    state: &mut VmRuntimeState,
    target_ram_bytes: u64,
    rollback: &mut Vec<HotplugRollback>,
) -> crate::Result<()> {
    if target_ram_bytes == state.ram_bytes {
        return Ok(());
    }

    info!(
        "Resizing VM memory from {} bytes to {} bytes",
        state.ram_bytes, target_ram_bytes
    );
    let previous_ram = state.ram_bytes;
    resize_vm(client, state.vcpus, target_ram_bytes).await?;
    state.ram_bytes = target_ram_bytes;
    rollback.push(HotplugRollback::ResizeMemory { previous_ram });
    Ok(())
}

async fn resize_vm(
    client: &mut ChApiClient,
    desired_vcpus: u64,
    desired_ram: u64,
) -> crate::Result<()> {
    client
        .put(
            "/api/v1/vm.resize",
            Some(&json!({
                "desired_vcpus": desired_vcpus,
                "desired_ram": desired_ram,
            })),
        )
        .await?;
    Ok(())
}

async fn apply_nic_add(client: &mut ChApiClient, config: &HotplugNetConfig) -> crate::Result<()> {
    info!("Hotplugging NIC {}", config.id);
    client
        .put(
            "/api/v1/vm.add-net",
            Some(&json!({
                "tap": config.tap,
                "mac": config.mac,
                "id": config.id,
            })),
        )
        .await?;
    Ok(())
}

async fn apply_nic_remove(client: &mut ChApiClient, id: &str) -> crate::Result<()> {
    let id = ch_nic_id(id);
    info!("Removing NIC {id}");
    client
        .put("/api/v1/vm.remove-device", Some(&json!({ "id": id })))
        .await?;
    Ok(())
}

async fn apply_disk_add(client: &mut ChApiClient, config: &HotplugDiskConfig) -> crate::Result<()> {
    info!("Hotplugging block volume {}", config.id);
    client
        .put(
            "/api/v1/vm.add-disk",
            Some(&json!({
                "path": config.path,
                "readonly": config.readonly,
                "id": config.id,
            })),
        )
        .await?;
    Ok(())
}

async fn apply_disk_remove(client: &mut ChApiClient, id: &str) -> crate::Result<()> {
    let id = ch_disk_id(id);
    info!("Removing block volume {id}");
    client
        .put("/api/v1/vm.remove-device", Some(&json!({ "id": id })))
        .await?;
    Ok(())
}

async fn execute_rollback(
    client: &mut ChApiClient,
    state: &mut VmRuntimeState,
    actions: Vec<HotplugRollback>,
) {
    for action in actions.into_iter().rev() {
        match action {
            HotplugRollback::ResizeCpu { previous_vcpus } => {
                if let Err(err) = resize_vm(client, previous_vcpus, state.ram_bytes).await {
                    warn!(
                        "hotplug rollback: failed to restore CPU count to {}: {}",
                        previous_vcpus, err
                    );
                } else {
                    state.vcpus = previous_vcpus;
                }
            }
            HotplugRollback::ResizeMemory { previous_ram } => {
                if let Err(err) = resize_vm(client, state.vcpus, previous_ram).await {
                    warn!(
                        "hotplug rollback: failed to restore memory to {} bytes: {}",
                        previous_ram, err
                    );
                } else {
                    state.ram_bytes = previous_ram;
                }
            }
            HotplugRollback::RemoveNic { id } => {
                if let Err(err) = apply_nic_remove(client, &id).await {
                    warn!("hotplug rollback: failed to remove NIC '{id}': {err}");
                }
            }
            HotplugRollback::AddNic { config } => {
                if let Err(err) = apply_nic_add(client, &config).await {
                    warn!(
                        "hotplug rollback: failed to restore NIC '{}': {err}",
                        config.id
                    );
                }
            }
            HotplugRollback::RemoveDisk { id } => {
                if let Err(err) = apply_disk_remove(client, &id).await {
                    warn!("hotplug rollback: failed to remove volume '{id}': {err}");
                }
            }
            HotplugRollback::AddDisk { config } => {
                if let Err(err) = apply_disk_add(client, &config).await {
                    warn!(
                        "hotplug rollback: failed to restore volume '{}': {err}",
                        config.id
                    );
                }
            }
        }
    }
}

fn find_nic_config_for_rollback(
    vm_info: &VmInfoResponse,
    requested_id: &str,
) -> crate::Result<HotplugNetConfig> {
    let key = normalize_identifier_key(requested_id, &["nic-", "net-"]);
    let net = vm_info
        .config
        .net
        .iter()
        .find(|net| net_config_key(net).is_some_and(|candidate| candidate == key))
        .ok_or_else(|| {
            crate::Error::ActionFailed(format!(
                "unable to find NIC '{requested_id}' in vm.info for rollback"
            ))
        })?;

    let tap = net.tap.clone().ok_or_else(|| {
        crate::Error::ActionFailed(format!(
            "NIC '{requested_id}' is missing a tap device in vm.info"
        ))
    })?;
    let mac = net.mac.clone().ok_or_else(|| {
        crate::Error::ActionFailed(format!(
            "NIC '{requested_id}' is missing a MAC address in vm.info"
        ))
    })?;

    Ok(HotplugNetConfig {
        tap,
        mac,
        id: format!("net-{key}"),
    })
}

fn find_disk_config_for_rollback(
    vm_info: &VmInfoResponse,
    requested_id: &str,
) -> crate::Result<HotplugDiskConfig> {
    let key = normalize_identifier_key(requested_id, &["dev-", "blk-"]);
    let disk = vm_info
        .config
        .disks
        .iter()
        .find(|disk| disk_config_key(disk).is_some_and(|candidate| candidate == key))
        .ok_or_else(|| {
            crate::Error::ActionFailed(format!(
                "unable to find volume '{requested_id}' in vm.info for rollback"
            ))
        })?;

    let path = disk.path.clone().ok_or_else(|| {
        crate::Error::ActionFailed(format!(
            "volume '{requested_id}' is missing a path in vm.info"
        ))
    })?;

    Ok(HotplugDiskConfig {
        path,
        readonly: disk.readonly,
        id: format!("blk-{key}"),
    })
}

fn net_config_key(net: &VmInfoNetConfig) -> Option<String> {
    net.id
        .as_deref()
        .map(|id| normalize_identifier_key(id, &["nic-", "net-"]))
        .or_else(|| net.mac.as_deref().map(sanitize_identifier))
}

fn disk_config_key(disk: &VmInfoDiskConfig) -> Option<String> {
    disk.id
        .as_deref()
        .map(|id| normalize_identifier_key(id, &["dev-", "blk-"]))
        .or_else(|| disk.path.as_deref().map(sanitize_identifier))
}

fn ch_nic_id(id: &str) -> String {
    format!("net-{}", normalize_identifier_key(id, &["nic-", "net-"]))
}

fn ch_disk_id(id: &str) -> String {
    format!("blk-{}", normalize_identifier_key(id, &["dev-", "blk-"]))
}

impl VmRuntimeState {
    fn from_vm_info(vm_info: &VmInfoResponse) -> Self {
        Self {
            vcpus: vm_info.config.cpus.boot_vcpus,
            ram_bytes: vm_info.config.memory.size,
        }
    }
}

impl HotplugNetConfig {
    fn from_network(network: &VmNetworkConfig) -> Self {
        let key = sanitize_identifier(&network.mac_address);
        Self {
            tap: network.iface_name.clone(),
            mac: network.mac_address.clone(),
            id: format!("net-{key}"),
        }
    }
}

impl HotplugDiskConfig {
    fn from_volume(volume: &VmVolumeConfig) -> crate::Result<Self> {
        if volume.kind != VmVolumeKind::Block {
            return Err(crate::Error::ActionFailed(
                "filesystem volume hotplug is not supported".into(),
            ));
        }

        let key = sanitize_identifier(&volume.host_path);
        Ok(Self {
            path: volume.host_path.clone(),
            readonly: volume.read_only,
            id: format!("blk-{key}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{HotplugArgs, ch_disk_id, ch_nic_id, hotplug_request, run};
    use crate::testing::{
        TestVm, decode_json, http_empty_response, http_json_response, spawn_mock_server,
    };
    use serde_json::{Value, json};
    use tugboat_vm_runtime_interface::hotplug::{
        VmCpuHotplugConfig, VmHotplugRequest, VmMemoryHotplugConfig, sanitize_identifier,
    };
    use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig, VmVolumeKind};

    #[tokio::test]
    async fn test_cpu_resize() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-cpu-resize");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response("200 OK", vm_info_body(2, 1024, vec![], vec![])),
                http_empty_response("204 No Content"),
            ],
        );

        hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: Some(VmCpuHotplugConfig { cores: 4 }),
                memory: None,
                nics_added: vec![],
                nics_removed: vec![],
                volumes_added: vec![],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/api/v1/vm.info");
        assert_eq!(
            decode_json(&requests[1].body),
            json!({
                "desired_vcpus": 4,
                "desired_ram": 1024,
            })
        );
    }

    #[tokio::test]
    async fn test_memory_resize() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-memory-resize");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response("200 OK", vm_info_body(2, 1024, vec![], vec![])),
                http_empty_response("204 No Content"),
            ],
        );

        hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: Some(VmMemoryHotplugConfig { size: 4096 }),
                nics_added: vec![],
                nics_removed: vec![],
                volumes_added: vec![],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            decode_json(&requests[1].body),
            json!({
                "desired_vcpus": 2,
                "desired_ram": 4096,
            })
        );
    }

    #[tokio::test]
    async fn test_nic_add() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-nic-add");
        let nic = VmNetworkConfig {
            iface_name: "tap0".into(),
            mac_address: "02:00:00:00:00:01".into(),
        };
        let expected_id = format!("net-{}", sanitize_identifier(&nic.mac_address));
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response("200 OK", vm_info_body(2, 1024, vec![], vec![])),
                http_empty_response("200 OK"),
            ],
        );

        hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: None,
                nics_added: vec![nic.clone()],
                nics_removed: vec![],
                volumes_added: vec![],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/api/v1/vm.add-net");
        assert_eq!(
            decode_json(&requests[1].body),
            json!({
                "tap": nic.iface_name,
                "mac": nic.mac_address,
                "id": expected_id,
            })
        );
    }

    #[tokio::test]
    async fn test_nic_remove() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-nic-remove");
        let mac = "02:00:00:00:00:02";
        let requested_id = format!("nic-{}", sanitize_identifier(mac));
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response(
                    "200 OK",
                    vm_info_body(
                        2,
                        1024,
                        vec![json!({
                            "tap": "tap1",
                            "mac": mac,
                        })],
                        vec![],
                    ),
                ),
                http_empty_response("204 No Content"),
            ],
        );

        hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: None,
                nics_added: vec![],
                nics_removed: vec![requested_id.clone()],
                volumes_added: vec![],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/api/v1/vm.remove-device");
        assert_eq!(
            decode_json(&requests[1].body),
            json!({ "id": ch_nic_id(&requested_id) })
        );
    }

    #[tokio::test]
    async fn test_volume_add() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-volume-add");
        let volume = VmVolumeConfig::block("/dev/sdb", "raw", false);
        let expected_id = format!("blk-{}", sanitize_identifier(&volume.host_path));
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response("200 OK", vm_info_body(2, 1024, vec![], vec![])),
                http_empty_response("200 OK"),
            ],
        );

        hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: None,
                nics_added: vec![],
                nics_removed: vec![],
                volumes_added: vec![volume.clone()],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/api/v1/vm.add-disk");
        assert_eq!(
            decode_json(&requests[1].body),
            json!({
                "path": volume.host_path,
                "readonly": false,
                "id": expected_id,
            })
        );
    }

    #[tokio::test]
    async fn test_volume_remove() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-volume-remove");
        let path = "/dev/sdc";
        let requested_id = format!("dev-{}", sanitize_identifier(path));
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response(
                    "200 OK",
                    vm_info_body(
                        2,
                        1024,
                        vec![],
                        vec![json!({
                            "path": path,
                            "readonly": true,
                        })],
                    ),
                ),
                http_empty_response("204 No Content"),
            ],
        );

        hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: None,
                nics_added: vec![],
                nics_removed: vec![],
                volumes_added: vec![],
                volumes_removed: vec![requested_id.clone()],
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/api/v1/vm.remove-device");
        assert_eq!(
            decode_json(&requests[1].body),
            json!({ "id": ch_disk_id(&requested_id) })
        );
    }

    #[tokio::test]
    async fn test_rollback_on_failure_restores_cpu_resize() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-cpu-rollback");
        let failing_nic = VmNetworkConfig {
            iface_name: "tap3".into(),
            mac_address: "02:00:00:00:00:03".into(),
        };
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response("200 OK", vm_info_body(2, 1024, vec![], vec![])),
                http_empty_response("204 No Content"),
                http_json_response("500 Internal Server Error", json!({ "error": "boom" })),
                http_empty_response("204 No Content"),
            ],
        );

        let error = hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: Some(VmCpuHotplugConfig { cores: 4 }),
                memory: None,
                nics_added: vec![failing_nic],
                nics_removed: vec![],
                volumes_added: vec![],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("500 Internal Server Error"));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            decode_json(&requests[1].body),
            json!({
                "desired_vcpus": 4,
                "desired_ram": 1024,
            })
        );
        assert_eq!(
            decode_json(&requests[3].body),
            json!({
                "desired_vcpus": 2,
                "desired_ram": 1024,
            })
        );
    }

    #[tokio::test]
    async fn test_rollback_on_failure_restores_removed_nic() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-nic-rollback");
        let existing_mac = "02:00:00:00:00:10";
        let existing_tap = "tap10";
        let remove_id = format!("nic-{}", sanitize_identifier(existing_mac));
        let failing_nic = VmNetworkConfig {
            iface_name: "tap11".into(),
            mac_address: "02:00:00:00:00:11".into(),
        };
        let expected_restore_id = format!("net-{}", sanitize_identifier(existing_mac));
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response(
                    "200 OK",
                    vm_info_body(
                        2,
                        1024,
                        vec![json!({
                            "tap": existing_tap,
                            "mac": existing_mac,
                        })],
                        vec![],
                    ),
                ),
                http_empty_response("204 No Content"),
                http_json_response("500 Internal Server Error", json!({ "error": "boom" })),
                http_empty_response("200 OK"),
            ],
        );

        let error = hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: None,
                nics_added: vec![failing_nic],
                nics_removed: vec![remove_id.clone()],
                volumes_added: vec![],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("500 Internal Server Error"));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            decode_json(&requests[1].body),
            json!({ "id": ch_nic_id(&remove_id) })
        );
        assert_eq!(
            decode_json(&requests[3].body),
            json!({
                "tap": existing_tap,
                "mac": existing_mac,
                "id": expected_restore_id,
            })
        );
    }

    #[tokio::test]
    async fn test_hotplug_reads_request_from_json_file() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-config-file");
        let config_path = test_vm.runtime_request_path("hotplug.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "id": test_vm.id,
                "cpu": { "cores": 3 },
                "nicsAdded": [],
                "nicsRemoved": [],
                "volumesAdded": [],
                "volumesRemoved": [],
            }))
            .unwrap(),
        )
        .unwrap();
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_json_response("200 OK", vm_info_body(2, 1024, vec![], vec![])),
                http_empty_response("204 No Content"),
            ],
        );

        run(
            test_vm.config.clone(),
            HotplugArgs {
                config: config_path.to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/api/v1/vm.resize");
        assert_eq!(
            decode_json(&requests[1].body),
            json!({
                "desired_vcpus": 3,
                "desired_ram": 1024,
            })
        );
    }

    #[tokio::test]
    async fn test_filesystem_volume_hotplug_is_rejected() {
        let test_vm = TestVm::new("tugboat-ch-hotplug", "vm-fs-hotplug");

        let error = hotplug_request(
            &test_vm.config,
            &VmHotplugRequest {
                id: test_vm.id.clone(),
                cpu: None,
                memory: None,
                nics_added: vec![],
                nics_removed: vec![],
                volumes_added: vec![VmVolumeConfig {
                    host_path: "/shared".into(),
                    kind: VmVolumeKind::Filesystem,
                    format: "raw".into(),
                    read_only: false,
                    mount_tag: "shared".into(),
                }],
                volumes_removed: vec![],
            },
        )
        .await
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("volumesAdded[0].kind must be block for hotplug")
        );
    }

    fn vm_info_body(vcpus: u64, memory_size: u64, net: Vec<Value>, disks: Vec<Value>) -> Value {
        json!({
            "config": {
                "cpus": {
                    "boot_vcpus": vcpus,
                },
                "memory": {
                    "size": memory_size,
                },
                "net": net,
                "disks": disks,
            },
            "state": "Running",
        })
    }
}
