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
use qapi::qmp::{MemoryBackendProperties, ObjectOptions};
use serde_json::json;
use tugboat_vm_runtime_interface::hotplug::VmMemoryHotplugRequest;

#[derive(Debug, Parser)]
pub struct HotplugMemoryArgs {
    #[arg(help = "Path to the hotplug memory request config file or - for stdin")]
    config: String,
}

pub async fn hotplug_memory(config: QemuVmConfig, args: HotplugMemoryArgs) -> crate::Result<()> {
    let req: VmMemoryHotplugRequest = load_config_or_panic(args.config);
    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&req.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();

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
                size: req.size_bytes_to_add,
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

    Ok(())
}
