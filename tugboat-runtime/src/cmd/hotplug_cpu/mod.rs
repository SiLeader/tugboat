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
use qapi::qmp::CpuInstanceProperties;
use serde_json::json;
use tugboat_vm_runtime_interface::hotplug::VmCpuHotplugRequest;

#[derive(Debug, Parser)]
pub struct HotplugCpuArgs {
    #[arg(help = "Path to the hotplug CPU request config file or - for stdin")]
    config: String,
}

pub async fn hotplug_cpu(config: QemuVmConfig, args: HotplugCpuArgs) -> crate::Result<()> {
    let req: VmCpuHotplugRequest = load_config_or_panic(args.config);
    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&req.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();

    let hotpluggable = qmp
        .execute(qapi::qmp::query_hotpluggable_cpus {})
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    let slots = hotpluggable
        .into_iter()
        .filter(|cpu| cpu.qom_path.is_none())
        .take(req.vcpus_to_add as usize)
        .collect::<Vec<_>>();

    if slots.len() != req.vcpus_to_add as usize {
        return Err(crate::Error::ActionFailed(
            "Not enough hotpluggable CPU slots available".to_string(),
        ));
    }

    for slot in slots {
        let arguments = build_cpu_arguments(&slot.props);
        // Use socket-id and core-id to create a unique device ID
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
