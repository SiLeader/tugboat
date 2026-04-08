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

use crate::config::load_config_or_panic;
use crate::execute::vm::QemuVmConfig;
use clap::Parser;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tugboat_vm_runtime_interface::hotplug::{
    VmHotplugRequest, VmMemoryHotplugConfig, normalize_identifier_key, sanitize_identifier,
};
use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig, VmVolumeKind};

#[derive(Debug, Parser)]
pub struct HotplugArgs {
    #[arg(help = "Path to the hotplug request config file or - for stdin")]
    config: String,
}

pub async fn run(config: QemuVmConfig, args: HotplugArgs) -> crate::Result<()> {
    let req: VmHotplugRequest = load_config_or_panic(args.config);
    let mut qmp = QmpClient::connect(config.get_uds_path(&req.id)).await?;

    if let Some(cpu) = &req.cpu {
        apply_cpu_hotplug(&mut qmp, cpu.cores).await?;
    }
    if let Some(memory) = &req.memory {
        apply_memory_hotplug(&mut qmp, memory).await?;
    }
    for nic in &req.nics_added {
        apply_nic_add(&mut qmp, nic).await?;
    }
    for nic in &req.nics_removed {
        apply_nic_remove(&mut qmp, nic).await?;
    }
    for volume in &req.volumes_added {
        apply_volume_add(&mut qmp, volume).await?;
    }
    for volume in &req.volumes_removed {
        apply_volume_remove(&mut qmp, volume).await?;
    }

    Ok(())
}

async fn apply_cpu_hotplug(qmp: &mut QmpClient, target_cores: u64) -> crate::Result<()> {
    let slots = qmp.execute("query-hotpluggable-cpus", None).await?;
    let slots = slots
        .as_array()
        .ok_or_else(|| crate::Error::Qmp("query-hotpluggable-cpus returned non-array".into()))?;

    let mut present = Vec::new();
    let mut absent = Vec::new();
    let mut current_cores = 0_u64;

    for slot in slots {
        if slot.get("qom-path").is_some() {
            current_cores += slot_u64(slot, "vcpus-count").unwrap_or(1);
            present.push(slot.clone());
        } else {
            absent.push(slot.clone());
        }
    }

    sort_cpu_slots(&mut present);
    sort_cpu_slots(&mut absent);

    match target_cores.cmp(&current_cores) {
        std::cmp::Ordering::Equal => {}
        std::cmp::Ordering::Greater => {
            let diff = (target_cores - current_cores) as usize;
            if absent.len() < diff {
                return Err(crate::Error::Qmp(format!(
                    "not enough hotpluggable CPU slots: need {diff}, have {}",
                    absent.len()
                )));
            }
            for slot in absent.iter().take(diff) {
                qmp.execute("device_add", Some(cpu_add_arguments(slot)?))
                    .await?;
            }
        }
        std::cmp::Ordering::Less => {
            let mut cores_to_remove = current_cores - target_cores;
            let mut removal_ids = Vec::new();
            for slot in present.iter().rev() {
                if cores_to_remove == 0 {
                    break;
                }
                let vcpus = slot_u64(slot, "vcpus-count").unwrap_or(1);
                if vcpus <= cores_to_remove {
                    removal_ids.push(cpu_slot_id(slot));
                    cores_to_remove -= vcpus;
                }
            }
            if cores_to_remove > 0 {
                return Err(crate::Error::Qmp(
                    "unable to satisfy requested CPU count exactly with available slots".into(),
                ));
            }
            for id in removal_ids {
                qmp.execute("device_del", Some(json!({ "id": id }))).await?;
            }
        }
    }

    Ok(())
}

async fn apply_memory_hotplug(
    qmp: &mut QmpClient,
    target: &VmMemoryHotplugConfig,
) -> crate::Result<()> {
    let current = current_memory_size(qmp).await?;
    match target.size.cmp(&current) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Greater => {
            let index = next_memory_index(qmp).await?;
            let backend_id = format!("mem-{index}");
            let dimm_id = format!("dimm-{index}");
            let size = target.size - current;

            qmp.execute(
                "object-add",
                Some(json!({
                    "qom-type": "memory-backend-ram",
                    "id": backend_id,
                    "size": size,
                })),
            )
            .await?;
            if let Err(err) = qmp
                .execute(
                    "device_add",
                    Some(json!({
                        "driver": "pc-dimm",
                        "id": dimm_id,
                        "memdev": backend_id,
                    })),
                )
                .await
            {
                let _ = qmp
                    .execute("object-del", Some(json!({ "id": backend_id })))
                    .await;
                return Err(err);
            }
            Ok(())
        }
        std::cmp::Ordering::Less => remove_memory_devices(qmp, current - target.size).await,
    }
}

async fn current_memory_size(qmp: &mut QmpClient) -> crate::Result<u64> {
    let summary = qmp.execute("query-memory-size-summary", None).await?;
    let base = summary
        .get("base-memory")
        .or_else(|| summary.get("base_memory"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let plugged = summary
        .get("plugged-memory")
        .or_else(|| summary.get("plugged_memory"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok(base + plugged)
}

async fn next_memory_index(qmp: &mut QmpClient) -> crate::Result<u64> {
    let devices = query_memory_devices(qmp).await?;
    Ok(devices
        .iter()
        .filter_map(|device| device_id(device, "id"))
        .filter_map(|id| id.strip_prefix("dimm-")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1)
}

async fn remove_memory_devices(qmp: &mut QmpClient, mut bytes_to_remove: u64) -> crate::Result<()> {
    let mut removable = query_memory_devices(qmp)
        .await?
        .into_iter()
        .filter_map(|device| {
            let id = device_id(&device, "id")?;
            let memdev = device_id(&device, "memdev")?;
            let size = device_size(&device)?;
            Some((id, memdev, size))
        })
        .collect::<Vec<_>>();
    removable.sort_by_key(|(_, _, size)| *size);

    let mut plan = Vec::new();
    for (id, memdev, size) in removable.into_iter().rev() {
        if bytes_to_remove == 0 {
            break;
        }
        if size <= bytes_to_remove {
            plan.push((id, memdev));
            bytes_to_remove -= size;
        }
    }

    if bytes_to_remove != 0 {
        return Err(crate::Error::Qmp(
            "unable to satisfy requested memory removal exactly".into(),
        ));
    }

    for (id, memdev) in plan {
        qmp.execute("device_del", Some(json!({ "id": id }))).await?;
        qmp.wait_for_device_deleted(&id).await?;
        qmp.execute("object-del", Some(json!({ "id": memdev })))
            .await?;
    }

    Ok(())
}

async fn query_memory_devices(qmp: &mut QmpClient) -> crate::Result<Vec<Value>> {
    let devices = qmp.execute("query-memory-devices", None).await?;
    devices
        .as_array()
        .cloned()
        .ok_or_else(|| crate::Error::Qmp("query-memory-devices returned non-array".into()))
}

async fn apply_nic_add(qmp: &mut QmpClient, nic: &VmNetworkConfig) -> crate::Result<()> {
    let key = nic_key(&nic.mac_address);
    qmp.execute(
        "netdev_add",
        Some(netdev_add_arguments(&key, &nic.iface_name)),
    )
    .await?;
    if let Err(err) = qmp
        .execute(
            "device_add",
            Some(nic_device_add_arguments(&key, &nic.mac_address)),
        )
        .await
    {
        let _ = qmp
            .execute("netdev_del", Some(json!({ "id": format!("net-{key}") })))
            .await;
        return Err(err);
    }
    Ok(())
}

async fn apply_nic_remove(qmp: &mut QmpClient, id: &str) -> crate::Result<()> {
    let key = normalize_identifier_key(id, &["nic-", "net-"]);
    let device_id = format!("nic-{key}");
    let netdev_id = format!("net-{key}");
    qmp.execute("device_del", Some(json!({ "id": device_id.clone() })))
        .await?;
    qmp.wait_for_device_deleted(&device_id).await?;
    qmp.execute("netdev_del", Some(json!({ "id": netdev_id })))
        .await?;
    Ok(())
}

async fn apply_volume_add(qmp: &mut QmpClient, volume: &VmVolumeConfig) -> crate::Result<()> {
    if volume.kind != VmVolumeKind::Block {
        return Err(crate::Error::ActionFailed(
            "filesystem volume hotplug is not supported".into(),
        ));
    }

    let key = volume_key(&volume.host_path);
    qmp.execute(
        "blockdev-add",
        Some(blockdev_add_arguments(&key, &volume.host_path)),
    )
    .await?;
    if let Err(err) = qmp
        .execute("device_add", Some(block_device_add_arguments(&key)))
        .await
    {
        let _ = qmp
            .execute(
                "blockdev-del",
                Some(json!({ "node-name": format!("blk-{key}") })),
            )
            .await;
        return Err(err);
    }

    Ok(())
}

async fn apply_volume_remove(qmp: &mut QmpClient, id: &str) -> crate::Result<()> {
    let key = normalize_identifier_key(id, &["dev-", "blk-"]);
    let device_id = format!("dev-{key}");
    let node_name = format!("blk-{key}");
    qmp.execute("device_del", Some(json!({ "id": device_id.clone() })))
        .await?;
    qmp.wait_for_device_deleted(&device_id).await?;
    qmp.execute("blockdev-del", Some(json!({ "node-name": node_name })))
        .await?;
    Ok(())
}

fn cpu_add_arguments(slot: &Value) -> crate::Result<Value> {
    let mut arguments = match slot {
        Value::Object(_) => Map::new(),
        _ => {
            return Err(crate::Error::Qmp(
                "query-hotpluggable-cpus entry returned non-object".into(),
            ));
        }
    };
    arguments.insert(
        "driver".into(),
        Value::String(
            slot.get("type")
                .and_then(Value::as_str)
                .unwrap_or("host-x86_64-cpu")
                .to_string(),
        ),
    );
    arguments.insert("id".into(), Value::String(cpu_slot_id(slot)));
    if let Some(props) = slot.get("props").and_then(Value::as_object) {
        for (key, value) in props {
            arguments.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::Object(arguments))
}

fn netdev_add_arguments(key: &str, iface_name: &str) -> Value {
    json!({
        "type": "tap",
        "id": format!("net-{key}"),
        "ifname": iface_name,
        "script": "no",
        "downscript": "no",
    })
}

fn nic_device_add_arguments(key: &str, mac_address: &str) -> Value {
    json!({
        "driver": "virtio-net-pci",
        "id": format!("nic-{key}"),
        "netdev": format!("net-{key}"),
        "mac": mac_address,
    })
}

fn blockdev_add_arguments(key: &str, host_path: &str) -> Value {
    json!({
        "driver": "raw",
        "node-name": format!("blk-{key}"),
        "file": {
            "driver": "file",
            "filename": host_path,
        },
    })
}

fn block_device_add_arguments(key: &str) -> Value {
    json!({
        "driver": "virtio-blk-pci",
        "id": format!("dev-{key}"),
        "drive": format!("blk-{key}"),
    })
}

fn sort_cpu_slots(slots: &mut [Value]) {
    slots.sort_by_key(|slot| {
        (
            slot_nested_u64(slot, "props", "socket-id").unwrap_or(0),
            slot_nested_u64(slot, "props", "core-id").unwrap_or(0),
            slot_nested_u64(slot, "props", "thread-id").unwrap_or(0),
        )
    });
}

fn cpu_slot_id(slot: &Value) -> String {
    format!(
        "cpu-{}-{}-{}",
        slot_nested_u64(slot, "props", "socket-id").unwrap_or(0),
        slot_nested_u64(slot, "props", "core-id").unwrap_or(0),
        slot_nested_u64(slot, "props", "thread-id").unwrap_or(0),
    )
}

fn slot_u64(slot: &Value, key: &str) -> Option<u64> {
    slot.get(key).and_then(Value::as_u64)
}

fn slot_nested_u64(slot: &Value, object_key: &str, key: &str) -> Option<u64> {
    slot.get(object_key)
        .and_then(Value::as_object)?
        .get(key)
        .and_then(Value::as_u64)
}

fn device_id(device: &Value, key: &str) -> Option<String> {
    device
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get(key))
        .or_else(|| device.get(key))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn device_size(device: &Value) -> Option<u64> {
    device
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("size"))
        .or_else(|| device.get("size"))
        .and_then(Value::as_u64)
}

fn nic_key(mac_address: &str) -> String {
    sanitize_identifier(mac_address)
}

fn volume_key(host_path: &str) -> String {
    sanitize_identifier(host_path)
}

struct QmpClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    pending_events: VecDeque<Value>,
}

impl QmpClient {
    async fn connect(path: String) -> crate::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (reader, writer) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(reader),
            writer,
            pending_events: VecDeque::new(),
        };
        client.read_message().await?;
        client.execute("qmp_capabilities", None).await?;
        Ok(client)
    }

    async fn execute(&mut self, command: &str, arguments: Option<Value>) -> crate::Result<Value> {
        let message = if let Some(arguments) = arguments {
            json!({
                "execute": command,
                "arguments": arguments,
            })
        } else {
            json!({
                "execute": command,
            })
        };
        let encoded = serde_json::to_vec(&message)
            .map_err(|e| crate::Error::Qmp(format!("failed to encode QMP request: {e}")))?;
        self.writer.write_all(&encoded).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;

        loop {
            let response = self.read_message().await?;
            if let Some(error) = response.get("error") {
                return Err(crate::Error::Qmp(error.to_string()));
            }
            if response.get("return").is_some() {
                return Ok(response.get("return").cloned().unwrap_or(Value::Null));
            }
            if response.get("event").is_some() {
                self.pending_events.push_back(response);
            }
        }
    }

    async fn wait_for_device_deleted(&mut self, id: &str) -> crate::Result<()> {
        if let Some(index) = self
            .pending_events
            .iter()
            .position(|event| device_deleted_event_matches(event, id))
        {
            self.pending_events.remove(index);
            return Ok(());
        }

        let wait = async {
            loop {
                let response = self.read_message().await?;
                if let Some(error) = response.get("error") {
                    return Err(crate::Error::Qmp(error.to_string()));
                }
                if device_deleted_event_matches(&response, id) {
                    return Ok(());
                }
                if response.get("event").is_some() {
                    self.pending_events.push_back(response);
                }
            }
        };

        tokio::time::timeout(Duration::from_secs(30), wait)
            .await
            .map_err(|_| {
                crate::Error::Qmp(format!(
                    "timed out waiting for DEVICE_DELETED event for '{id}'"
                ))
            })?
    }

    async fn read_message(&mut self) -> crate::Result<Value> {
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line).await?;
            if bytes == 0 {
                return Err(crate::Error::Qmp("unexpected EOF from QMP socket".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(trimmed)
                .map_err(|e| crate::Error::Qmp(format!("invalid QMP response: {e}")))?;
            return Ok(value);
        }
    }
}

fn device_deleted_event_matches(message: &Value, id: &str) -> bool {
    message.get("event").and_then(Value::as_str) == Some("DEVICE_DELETED")
        && message
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("device"))
            .and_then(Value::as_str)
            == Some(id)
}

#[cfg(test)]
mod tests {
    use super::{
        QmpClient, apply_cpu_hotplug, apply_nic_remove, apply_volume_add, apply_volume_remove,
        block_device_add_arguments, blockdev_add_arguments, cpu_add_arguments,
        netdev_add_arguments, nic_device_add_arguments, remove_memory_devices,
    };
    use serde_json::{Value, json};
    use std::future::Future;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;
    use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
    use tugboat_vm_runtime_interface::run::VmVolumeConfig;

    #[test]
    fn cpu_add_command_format() {
        let slot = json!({
            "type": "host-x86_64-cpu",
            "props": {
                "socket-id": 0,
                "core-id": 2,
                "thread-id": 0,
            }
        });

        assert_eq!(
            cpu_add_arguments(&slot).unwrap(),
            json!({
                "driver": "host-x86_64-cpu",
                "id": "cpu-0-2-0",
                "socket-id": 0,
                "core-id": 2,
                "thread-id": 0,
            })
        );
    }

    #[test]
    fn nic_add_commands_format() {
        assert_eq!(
            netdev_add_arguments("02-00-00-00-00-01", "tap0"),
            json!({
                "type": "tap",
                "id": "net-02-00-00-00-00-01",
                "ifname": "tap0",
                "script": "no",
                "downscript": "no",
            })
        );
        assert_eq!(
            nic_device_add_arguments("02-00-00-00-00-01", "02:00:00:00:00:01"),
            json!({
                "driver": "virtio-net-pci",
                "id": "nic-02-00-00-00-00-01",
                "netdev": "net-02-00-00-00-00-01",
                "mac": "02:00:00:00:00:01",
            })
        );
    }

    #[test]
    fn block_add_commands_format() {
        assert_eq!(
            blockdev_add_arguments("var-lib-disk1-img", "/var/lib/disk1.img"),
            json!({
                "driver": "raw",
                "node-name": "blk-var-lib-disk1-img",
                "file": {
                    "driver": "file",
                    "filename": "/var/lib/disk1.img",
                },
            })
        );
        assert_eq!(
            block_device_add_arguments("var-lib-disk1-img"),
            json!({
                "driver": "virtio-blk-pci",
                "id": "dev-var-lib-disk1-img",
                "drive": "blk-var-lib-disk1-img",
            })
        );
    }

    #[tokio::test]
    async fn cpu_hotplug_reduction_removes_present_cpu_slots() {
        let (socket_path, server) =
            spawn_fake_qmp_server("cpu-remove", |mut reader, mut writer| async move {
                qmp_handshake(&mut reader, &mut writer).await;

                expect_command(&mut reader, "query-hotpluggable-cpus").await;
                write_json(
                    &mut writer,
                    json!({
                        "return": [
                            {
                                "type": "host-x86_64-cpu",
                                "qom-path": "/machine/peripheral/cpu-0-0-0",
                                "vcpus-count": 1,
                                "props": {
                                    "socket-id": 0,
                                    "core-id": 0,
                                    "thread-id": 0
                                }
                            },
                            {
                                "type": "host-x86_64-cpu",
                                "qom-path": "/machine/peripheral/cpu-0-1-0",
                                "vcpus-count": 1,
                                "props": {
                                    "socket-id": 0,
                                    "core-id": 1,
                                    "thread-id": 0
                                }
                            },
                            {
                                "type": "host-x86_64-cpu",
                                "vcpus-count": 1,
                                "props": {
                                    "socket-id": 0,
                                    "core-id": 2,
                                    "thread-id": 0
                                }
                            }
                        ]
                    }),
                )
                .await;

                let request = expect_command(&mut reader, "device_del").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&json!({ "id": "cpu-0-1-0" }))
                );
                write_json(&mut writer, json!({ "return": {} })).await;

                assert_no_extra_commands(&mut reader).await;
            })
            .await;

        let mut qmp = QmpClient::connect(socket_path).await.unwrap();
        apply_cpu_hotplug(&mut qmp, 1).await.unwrap();
        drop(qmp);

        server.await.unwrap();
    }

    #[tokio::test]
    async fn memory_remove_waits_for_device_deleted_before_backend_cleanup() {
        let (socket_path, server) =
            spawn_fake_qmp_server("memory-remove", |mut reader, mut writer| async move {
                qmp_handshake(&mut reader, &mut writer).await;

                expect_command(&mut reader, "query-memory-devices").await;
                write_json(
                    &mut writer,
                    json!({
                        "return": [
                            {
                                "data": {
                                    "id": "dimm-1",
                                    "memdev": "mem-1",
                                    "size": 1024
                                }
                            }
                        ]
                    }),
                )
                .await;

                let request = expect_command(&mut reader, "device_del").await;
                assert_eq!(request.get("arguments"), Some(&json!({ "id": "dimm-1" })));
                write_json(&mut writer, json!({ "return": {} })).await;
                assert_no_extra_commands(&mut reader).await;
                write_json(
                    &mut writer,
                    json!({
                        "event": "DEVICE_DELETED",
                        "data": { "device": "dimm-1" }
                    }),
                )
                .await;

                let request = expect_command(&mut reader, "object-del").await;
                assert_eq!(request.get("arguments"), Some(&json!({ "id": "mem-1" })));
                write_json(&mut writer, json!({ "return": {} })).await;

                assert_no_extra_commands(&mut reader).await;
            })
            .await;

        let mut qmp = QmpClient::connect(socket_path).await.unwrap();
        remove_memory_devices(&mut qmp, 1024).await.unwrap();
        drop(qmp);

        server.await.unwrap();
    }

    #[tokio::test]
    async fn nic_remove_waits_for_device_deleted_before_backend_cleanup() {
        let nic_id = "02:00:00:00:00:01";
        let key = super::nic_key(nic_id);
        let (socket_path, server) =
            spawn_fake_qmp_server("nic-remove", move |mut reader, mut writer| async move {
                qmp_handshake(&mut reader, &mut writer).await;

                let request = expect_command(&mut reader, "device_del").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&json!({ "id": format!("nic-{key}") }))
                );
                write_json(&mut writer, json!({ "return": {} })).await;
                assert_no_extra_commands(&mut reader).await;
                write_json(
                    &mut writer,
                    json!({
                        "event": "DEVICE_DELETED",
                        "data": { "device": format!("nic-{key}") }
                    }),
                )
                .await;

                let request = expect_command(&mut reader, "netdev_del").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&json!({ "id": format!("net-{key}") }))
                );
                write_json(&mut writer, json!({ "return": {} })).await;

                assert_no_extra_commands(&mut reader).await;
            })
            .await;

        let mut qmp = QmpClient::connect(socket_path).await.unwrap();
        apply_nic_remove(&mut qmp, nic_id).await.unwrap();
        drop(qmp);

        server.await.unwrap();
    }

    #[tokio::test]
    async fn volume_add_executes_block_setup_once() {
        let volume = VmVolumeConfig::block("/var/lib/disk1.img", "raw", false);
        let key = super::volume_key(&volume.host_path);
        let volume_for_server = volume.clone();
        let (socket_path, server) =
            spawn_fake_qmp_server("volume-add", move |mut reader, mut writer| async move {
                qmp_handshake(&mut reader, &mut writer).await;

                let request = expect_command(&mut reader, "blockdev-add").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&blockdev_add_arguments(&key, &volume_for_server.host_path))
                );
                write_json(&mut writer, json!({ "return": {} })).await;

                let request = expect_command(&mut reader, "device_add").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&block_device_add_arguments(&key))
                );
                write_json(&mut writer, json!({ "return": {} })).await;

                assert_no_extra_commands(&mut reader).await;
            })
            .await;

        let mut qmp = QmpClient::connect(socket_path).await.unwrap();
        apply_volume_add(&mut qmp, &volume).await.unwrap();
        drop(qmp);

        server.await.unwrap();
    }

    #[tokio::test]
    async fn volume_remove_waits_for_device_deleted_before_backend_cleanup() {
        let volume_id = "/var/lib/disk1.img";
        let key = super::volume_key(volume_id);
        let (socket_path, server) =
            spawn_fake_qmp_server("volume-remove", move |mut reader, mut writer| async move {
                qmp_handshake(&mut reader, &mut writer).await;

                let request = expect_command(&mut reader, "device_del").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&json!({ "id": format!("dev-{key}") }))
                );
                write_json(&mut writer, json!({ "return": {} })).await;
                assert_no_extra_commands(&mut reader).await;
                write_json(
                    &mut writer,
                    json!({
                        "event": "DEVICE_DELETED",
                        "data": { "device": format!("dev-{key}") }
                    }),
                )
                .await;

                let request = expect_command(&mut reader, "blockdev-del").await;
                assert_eq!(
                    request.get("arguments"),
                    Some(&json!({ "node-name": format!("blk-{key}") }))
                );
                write_json(&mut writer, json!({ "return": {} })).await;

                assert_no_extra_commands(&mut reader).await;
            })
            .await;

        let mut qmp = QmpClient::connect(socket_path).await.unwrap();
        apply_volume_remove(&mut qmp, volume_id).await.unwrap();
        drop(qmp);

        server.await.unwrap();
    }

    fn unique_socket_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tugboat-runtime-hotplug-{name}-{}.sock",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be monotonic")
                .as_nanos()
        ))
    }

    async fn spawn_fake_qmp_server<F, Fut>(
        name: &str,
        handler: F,
    ) -> (String, tokio::task::JoinHandle<()>)
    where
        F: FnOnce(BufReader<OwnedReadHalf>, OwnedWriteHalf) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let socket_path = unique_socket_path(name);
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("listener should bind");
        let socket_path_for_server = socket_path.clone();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("server should accept");
            let (reader, writer) = stream.into_split();
            handler(BufReader::new(reader), writer).await;
            let _ = std::fs::remove_file(socket_path_for_server);
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        (socket_path.display().to_string(), handle)
    }

    async fn qmp_handshake(reader: &mut BufReader<OwnedReadHalf>, writer: &mut OwnedWriteHalf) {
        write_json(
            writer,
            json!({
                "QMP": {
                    "version": {
                        "qemu": {
                            "major": 9,
                            "minor": 0,
                            "micro": 0
                        },
                        "package": ""
                    },
                    "capabilities": []
                }
            }),
        )
        .await;
        let request = expect_command(reader, "qmp_capabilities").await;
        assert!(request.get("arguments").is_none());
        write_json(writer, json!({ "return": {} })).await;
    }

    async fn expect_command(reader: &mut BufReader<OwnedReadHalf>, expected: &str) -> Value {
        let request = read_json(reader).await;
        assert_eq!(
            request.get("execute").and_then(Value::as_str),
            Some(expected)
        );
        request
    }

    async fn read_json(reader: &mut BufReader<OwnedReadHalf>) -> Value {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .await
            .expect("request should be readable");
        assert!(bytes > 0, "expected QMP message");
        serde_json::from_str(line.trim()).expect("request should be valid JSON")
    }

    async fn write_json(writer: &mut OwnedWriteHalf, value: Value) {
        let encoded = serde_json::to_vec(&value).expect("response should encode");
        writer
            .write_all(&encoded)
            .await
            .expect("response should write");
        writer
            .write_all(b"\n")
            .await
            .expect("response newline should write");
        writer.flush().await.expect("response should flush");
    }

    async fn assert_no_extra_commands(reader: &mut BufReader<OwnedReadHalf>) {
        let mut line = String::new();
        match tokio::time::timeout(Duration::from_millis(100), reader.read_line(&mut line)).await {
            Err(_) => {}
            Ok(Ok(0)) => {}
            Ok(Ok(_)) => panic!("unexpected extra QMP command: {}", line.trim()),
            Ok(Err(err)) => panic!("failed to read trailing QMP command: {err}"),
        }
    }
}
