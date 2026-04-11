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

use crate::cmd::qmp::{connect_qmp, execute_with_timeout};
use crate::config::load_config;
use crate::execute::vm::QemuVmConfig;
use clap::Parser;
use qapi::qmp::{
    MigrateSetParameters, MigrationCapability, MigrationCapabilityStatus, MigrationStatus,
    ZeroPageDetection, migrate_set_capabilities, migrate_set_parameters, migrate_start_postcopy,
    query_migrate,
};
use tokio::time::{Duration, Instant, sleep};
use tugboat_vm_runtime_interface::migrate::VmMigrateRequest;

const MAX_MIGRATION_BANDWIDTH_BYTES_PER_SEC: u64 = 1 << 30;
const MIGRATION_DOWNTIME_LIMIT_MS: u64 = 300;
const XBZRLE_CACHE_SIZE_BYTES: u64 = 64 << 20;
const POSTCOPY_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const MIGRATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Parser)]
pub struct MigrateArgs {
    #[arg(help = "Path to the migrate request config file or - for stdin")]
    config: String,
}

pub async fn migrate(config: QemuVmConfig, args: MigrateArgs) -> crate::Result<()> {
    let req: VmMigrateRequest = load_config(args.config)?;
    crate::validate::validate_safe_id(&req.id, "vm id")?;
    let (qmp, _handle) = connect_qmp(config.get_uds_path(&req.id))
        .await?
        .spawn_tokio();

    let mut capabilities = vec![
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
    ];
    if req.postcopy_enabled {
        capabilities.push(MigrationCapabilityStatus {
            capability: MigrationCapability::postcopy_ram,
            state: true,
        });
    }

    execute_with_timeout(
        qmp.execute(migrate_set_capabilities { capabilities }),
        "Timed out setting migration capabilities",
    )
    .await?;

    execute_with_timeout(
        qmp.execute(migrate_set_parameters(MigrateSetParameters {
            max_bandwidth: Some(
                req.max_bandwidth_bytes_per_sec
                    .unwrap_or(MAX_MIGRATION_BANDWIDTH_BYTES_PER_SEC),
            ),
            downtime_limit: Some(req.downtime_limit_ms.unwrap_or(MIGRATION_DOWNTIME_LIMIT_MS)),
            xbzrle_cache_size: Some(
                req.xbzrle_cache_size_bytes
                    .unwrap_or(XBZRLE_CACHE_SIZE_BYTES),
            ),
            zero_page_detection: Some(ZeroPageDetection::legacy),
            ..Default::default()
        })),
        "Timed out setting migration parameters",
    )
    .await?;

    execute_with_timeout(
        qmp.execute(qapi::qmp::migrate {
            uri: Some(format!(
                "tcp:{}:{}",
                req.destination_address, req.destination_port
            )),
            channels: None,
            detach: None,
            resume: None,
        }),
        "Timed out starting migration",
    )
    .await?;

    if req.postcopy_enabled {
        let deadline = Instant::now() + POSTCOPY_WAIT_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(crate::Error::Qmp(
                    "Timed out waiting for migration to reach active state before postcopy"
                        .to_string(),
                ));
            }
            let info = execute_with_timeout(
                qmp.execute(query_migrate {}),
                "Timed out querying migration status",
            )
            .await?;
            match info.status {
                Some(MigrationStatus::active) => break,
                Some(
                    MigrationStatus::completed
                    | MigrationStatus::failed
                    | MigrationStatus::cancelled
                    | MigrationStatus::cancelling,
                ) => return Ok(()),
                _ => sleep(MIGRATION_POLL_INTERVAL).await,
            }
        }
        execute_with_timeout(
            qmp.execute(migrate_start_postcopy {}),
            "Timed out starting postcopy migration",
        )
        .await?;
    }

    Ok(())
}
