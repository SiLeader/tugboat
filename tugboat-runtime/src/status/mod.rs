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

use crate::run::QemuVmConfig;
use clap::Parser;
use qapi::futures::QmpStreamTokio;
use qapi::qmp::RunState;
use tugboat_vm_runtime_interface::status::{
    VmErrorReason, VmPausedReason, VmRunningStatus, VmStatus,
};

#[derive(Debug, Parser)]
pub(crate) struct StatusArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

trait FromQmp<T> {
    fn from_qmp(value: T) -> Self;
}

impl FromQmp<RunState> for VmRunningStatus {
    fn from_qmp(value: RunState) -> Self {
        match value {
            RunState::debug => VmRunningStatus::Running,
            RunState::inmigrate => VmRunningStatus::Paused(VmPausedReason::InMigrating),
            RunState::internal_error => VmRunningStatus::Error(VmErrorReason::InternalError),
            RunState::io_error => VmRunningStatus::Error(VmErrorReason::IoError),
            RunState::paused => VmRunningStatus::Paused(VmPausedReason::Stopped),
            RunState::postmigrate => VmRunningStatus::Paused(VmPausedReason::PostMigration),
            RunState::prelaunch => VmRunningStatus::Prelaunch,
            RunState::finish_migrate => VmRunningStatus::Paused(VmPausedReason::FinishMigrating),
            RunState::restore_vm => VmRunningStatus::Paused(VmPausedReason::Restoring),
            RunState::running => VmRunningStatus::Running,
            RunState::save_vm => VmRunningStatus::Paused(VmPausedReason::Saving),
            RunState::shutdown => VmRunningStatus::Shutdown,
            RunState::suspended => VmRunningStatus::Suspended,
            RunState::watchdog => VmRunningStatus::Paused(VmPausedReason::Watchdog),
            RunState::guest_panicked => VmRunningStatus::Panicked,
            RunState::colo => VmRunningStatus::Paused(VmPausedReason::Saving),
        }
    }
}

pub(crate) async fn status(vm: QemuVmConfig, args: StatusArgs) {
    let stream = QmpStreamTokio::open_uds(vm.get_uds_path(&args.id))
        .await
        .expect("Cannot open UDS");
    let stream = stream.negotiate().await.expect("Cannot negotiate QMP");
    let (qmp, _handle) = stream.spawn_tokio();
    let status = qmp
        .execute(qapi::qmp::query_status {})
        .await
        .expect("Cannot execute QMP");
    let status = VmStatus {
        status: VmRunningStatus::from_qmp(status.status),
    };
    serde_json::to_writer(std::io::stdout(), &status).expect("Cannot serialize status");
}
