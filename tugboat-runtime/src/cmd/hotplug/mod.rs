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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tugboat_vm_runtime_interface::hotplug::{VmHotplugRequest, VmMemoryHotplugConfig};
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
            let diff = (current_cores - target_cores) as usize;
            if present.len() < diff {
                return Err(crate::Error::Qmp(format!(
                    "not enough present CPUs to remove: need {diff}, have {}",
                    present.len()
                )));
            }
            for slot in present.iter().rev().take(diff) {
                qmp.execute("device_del", Some(json!({ "id": cpu_slot_id(slot) })))
                    .await?;
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
            qmp.execute(
                "device_add",
                Some(json!({
                    "driver": "pc-dimm",
                    "id": dimm_id,
                    "memdev": backend_id,
                })),
            )
            .await?;
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
    qmp.execute(
        "device_add",
        Some(nic_device_add_arguments(&key, &nic.mac_address)),
    )
    .await?;
    Ok(())
}

async fn apply_nic_remove(qmp: &mut QmpClient, id: &str) -> crate::Result<()> {
    let key = normalize_existing_key(id, &["nic-", "net-"]);
    qmp.execute("device_del", Some(json!({ "id": format!("nic-{key}") })))
        .await?;
    qmp.execute("netdev_del", Some(json!({ "id": format!("net-{key}") })))
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
    qmp.execute("device_add", Some(block_device_add_arguments(&key)))
        .await?;
    Ok(())
}

async fn apply_volume_remove(qmp: &mut QmpClient, id: &str) -> crate::Result<()> {
    let key = normalize_existing_key(id, &["dev-", "blk-"]);
    qmp.execute("device_del", Some(json!({ "id": format!("dev-{key}") })))
        .await?;
    qmp.execute(
        "blockdev-del",
        Some(json!({ "node-name": format!("blk-{key}") })),
    )
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

fn normalize_existing_key(value: &str, prefixes: &[&str]) -> String {
    let trimmed = prefixes
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .unwrap_or(value);
    sanitize_identifier(trimmed)
}

fn sanitize_identifier(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

struct QmpClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl QmpClient {
    async fn connect(path: String) -> crate::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (reader, writer) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(reader),
            writer,
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
        }
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

#[cfg(test)]
mod tests {
    use super::{
        block_device_add_arguments, blockdev_add_arguments, cpu_add_arguments,
        netdev_add_arguments, nic_device_add_arguments,
    };
    use serde_json::json;

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
}
