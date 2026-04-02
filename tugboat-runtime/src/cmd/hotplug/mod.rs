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
use qapi::Dictionary;
use qapi::futures::QmpStreamTokio;
use qapi::qmp::{CpuInstanceProperties, MemoryBackendProperties, ObjectOptions};
use serde_json::json;
use tugboat_vm_runtime_interface::hotplug::VmHotplugRequest;

#[derive(Debug, Parser)]
pub struct HotplugArgs {
    #[arg(help = "Path to the hotplug request config file or - for stdin")]
    config: String,
}

pub async fn hotplug(config: QemuVmConfig, args: HotplugArgs) -> crate::Result<()> {
    let req: VmHotplugRequest = load_config_or_panic(args.config);
    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&req.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();

    if let Some(vcpus_total) = req.vcpus_to_add && vcpus_total > 0 {
        // determine current vcpu count: prefer agent-supplied value if present
        let current_vcpus = if let Some(cur) = req.current_vcpus {
            cur
        } else {
            let hotpluggable = qmp
                .execute(qapi::qmp::query_hotpluggable_cpus {})
                .await
                .map_err(|e| crate::Error::Qmp(e.to_string()))?;
            hotpluggable.iter().filter(|cpu| cpu.qom_path.is_some()).count() as u64
        };

        if vcpus_total < current_vcpus {
            return Err(crate::Error::ActionFailed(
                format!("CPU decrease from {} to {} is not supported", current_vcpus, vcpus_total),
            ));
        }

        let to_add = (vcpus_total - current_vcpus) as usize;
        if to_add > 0 {
            let hotpluggable = qmp
                .execute(qapi::qmp::query_hotpluggable_cpus {})
                .await
                .map_err(|e| crate::Error::Qmp(e.to_string()))?;

            let slots = hotpluggable
                .into_iter()
                .filter(|cpu| cpu.qom_path.is_none())
                .take(to_add)
                .collect::<Vec<_>>();

            if slots.len() != to_add {
                return Err(crate::Error::ActionFailed(
                    "Not enough hotpluggable CPU slots available".to_string(),
                ));
            }

            for slot in slots {
                let arguments = build_cpu_arguments(&slot.props);
                let device_id = format!(
                    "cpu-{}-{}",
                    slot.props.socket_id.unwrap_or(0),
                    slot.props.core_id.unwrap_or(0)
                );
                qmp.execute(qapi::qmp::device_add {
                    bus: None,
                    id: Some(device_id),
                    driver: slot.type_,
                    arguments,
                })
                .await
                .map_err(|e| crate::Error::Qmp(e.to_string()))?;
            }
        }
    }

    if let Some(size_bytes_total) = req.size_bytes_to_add && size_bytes_total > 0 {
        let current_size = req.current_size_bytes.unwrap_or(0u64);
        if size_bytes_total < current_size {
            return Err(crate::Error::ActionFailed(format!(
                "memory decrease from {} to {} is not supported",
                current_size, size_bytes_total
            )));
        }

        let to_add = size_bytes_total - current_size;
        if to_add > 0 {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let memdev_id = format!("mem-hotplug-{ts}");
            let dimm_id = format!("dimm-hotplug-{ts}");

            qmp.execute(qapi::qmp::object_add::from(
                ObjectOptions::memory_backend_ram {
                    id: memdev_id.clone(),
                    memory_backend_ram: MemoryBackendProperties {
                        dump: None,
                        host_nodes: None,
                        merge: None,
                        policy: None,
                        prealloc: None,
                        prealloc_context: None,
                        prealloc_threads: None,
                        reserve: None,
                        share: None,
                        x_use_canonical_path_for_ramblock_id: None,
                        size: to_add,
                    },
                },
            ))
            .await
            .map_err(|e| crate::Error::Qmp(e.to_string()))?;

            let mut arguments = Dictionary::new();
            arguments.insert("memdev".to_string(), json!(memdev_id));
            qmp.execute(qapi::qmp::device_add {
                bus: None,
                id: Some(dimm_id),
                driver: "pc-dimm".to_string(),
                arguments,
            })
            .await
            .map_err(|e| crate::Error::Qmp(e.to_string()))?;
        }
    }

    Ok(())
}

fn build_cpu_arguments(props: &CpuInstanceProperties) -> Dictionary {
    let mut arguments = Dictionary::new();
    if let Some(socket_id) = props.socket_id {
        arguments.insert("socket-id".to_string(), json!(socket_id));
    }
    if let Some(die_id) = props.die_id {
        arguments.insert("die-id".to_string(), json!(die_id));
    }
    if let Some(cluster_id) = props.cluster_id {
        arguments.insert("cluster-id".to_string(), json!(cluster_id));
    }
    if let Some(module_id) = props.module_id {
        arguments.insert("module-id".to_string(), json!(module_id));
    }
    if let Some(core_id) = props.core_id {
        arguments.insert("core-id".to_string(), json!(core_id));
    }
    if let Some(thread_id) = props.thread_id {
        arguments.insert("thread-id".to_string(), json!(thread_id));
    }
    if let Some(node_id) = props.node_id {
        arguments.insert("node-id".to_string(), json!(node_id));
    }
    if let Some(book_id) = props.book_id {
        arguments.insert("book-id".to_string(), json!(book_id));
    }
    if let Some(drawer_id) = props.drawer_id {
        arguments.insert("drawer-id".to_string(), json!(drawer_id));
    }
    arguments
}
