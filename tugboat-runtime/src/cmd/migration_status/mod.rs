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

use crate::execute::vm::QemuVmConfig;
use clap::Parser;
use qapi::futures::QmpStreamTokio;
use qapi::qmp::MigrationStatus;
use tugboat_vm_runtime_interface::migrate::{VmMigrationPhase, VmMigrationStatusResponse};

#[derive(Debug, Parser)]
pub struct MigrationStatusArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

pub async fn status(config: QemuVmConfig, args: MigrationStatusArgs) -> crate::Result<()> {
    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&args.id))
        .await
        .expect("Cannot open UDS");
    let stream = stream.negotiate().await.expect("Cannot negotiate QMP");
    let (qmp, _handle) = stream.spawn_tokio();

    let migration = qmp
        .execute(qapi::qmp::query_migrate {})
        .await
        .expect("Cannot execute QMP");

    let response = VmMigrationStatusResponse {
        phase: match migration.status {
            None | Some(MigrationStatus::none) => VmMigrationPhase::None,
            Some(MigrationStatus::setup) => VmMigrationPhase::Setup,
            Some(MigrationStatus::active)
            | Some(MigrationStatus::postcopy_active)
            | Some(MigrationStatus::postcopy_paused)
            | Some(MigrationStatus::postcopy_recover)
            | Some(MigrationStatus::postcopy_recover_setup)
            | Some(MigrationStatus::pre_switchover)
            | Some(MigrationStatus::device)
            | Some(MigrationStatus::wait_unplug)
            | Some(MigrationStatus::colo)
            | Some(MigrationStatus::cancelling) => VmMigrationPhase::Active,
            Some(MigrationStatus::completed) => VmMigrationPhase::Completed,
            Some(MigrationStatus::failed) => VmMigrationPhase::Failed,
            Some(MigrationStatus::cancelled) => VmMigrationPhase::Cancelled,
        },
        message: migration
            .error_desc
            .or_else(|| migration.status.map(|status| format!("{status:?}")))
            .unwrap_or_else(|| "Migration is not active.".to_string()),
    };

    serde_json::to_writer(std::io::stdout(), &response).expect("Cannot serialize status");
    Ok(())
}
