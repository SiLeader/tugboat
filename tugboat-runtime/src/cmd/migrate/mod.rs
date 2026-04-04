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
use qapi::futures::QmpStreamTokio;
use qapi::qmp::{
    MigrateSetParameters, MigrationCapability, MigrationCapabilityStatus, ZeroPageDetection,
    migrate_set_capabilities, migrate_set_parameters,
};
use tugboat_vm_runtime_interface::migrate::VmMigrateRequest;

const MAX_MIGRATION_BANDWIDTH_BYTES_PER_SEC: u64 = 1 << 30;
const MIGRATION_DOWNTIME_LIMIT_MS: u64 = 300;
const XBZRLE_CACHE_SIZE_BYTES: u64 = 64 << 20;

#[derive(Debug, Parser)]
pub struct MigrateArgs {
    #[arg(help = "Path to the migrate request config file or - for stdin")]
    config: String,
}

pub async fn migrate(config: QemuVmConfig, args: MigrateArgs) -> crate::Result<()> {
    let req: VmMigrateRequest = load_config_or_panic(args.config);
    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&req.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();

    qmp.execute(migrate_set_capabilities {
        capabilities: vec![
            MigrationCapabilityStatus {
                capability: MigrationCapability::events,
                state: true,
            },
            MigrationCapabilityStatus {
                capability: MigrationCapability::auto_converge,
                state: true,
            },
            MigrationCapabilityStatus {
                capability: MigrationCapability::xbzrle,
                state: true,
            },
        ],
    })
    .await
    .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    qmp.execute(migrate_set_parameters(MigrateSetParameters {
        max_bandwidth: Some(MAX_MIGRATION_BANDWIDTH_BYTES_PER_SEC),
        downtime_limit: Some(MIGRATION_DOWNTIME_LIMIT_MS),
        xbzrle_cache_size: Some(XBZRLE_CACHE_SIZE_BYTES),
        zero_page_detection: Some(ZeroPageDetection::legacy),
        ..Default::default()
    }))
    .await
    .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    qmp.execute(qapi::qmp::migrate {
        uri: Some(format!(
            "tcp:{}:{}",
            req.destination_address, req.destination_port
        )),
        channels: None,
        detach: None,
        resume: None,
    })
    .await
    .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    Ok(())
}
