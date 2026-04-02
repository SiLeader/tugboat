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
use qapi::qmp::RunState;
use tugboat_vm_runtime_interface::status::{VmStatus, VmStatusResponse};

#[derive(Debug, Parser)]
pub(crate) struct StatusArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

trait FromQmp<T> {
    fn from_qmp(value: T) -> Self;
}

impl FromQmp<RunState> for VmStatus {
    fn from_qmp(value: RunState) -> Self {
        match value {
            RunState::debug => VmStatus::Running,
            RunState::inmigrate => VmStatus::Paused,
            RunState::internal_error => VmStatus::Error,
            RunState::io_error => VmStatus::Error,
            RunState::paused => VmStatus::Paused,
            RunState::postmigrate => VmStatus::Paused,
            RunState::prelaunch => VmStatus::Prelaunch,
            RunState::finish_migrate => VmStatus::Paused,
            RunState::restore_vm => VmStatus::Paused,
            RunState::running => VmStatus::Running,
            RunState::save_vm => VmStatus::Paused,
            RunState::shutdown => VmStatus::Shutdown,
            RunState::suspended => VmStatus::Suspended,
            RunState::watchdog => VmStatus::Paused,
            RunState::guest_panicked => VmStatus::Panicked,
            RunState::colo => VmStatus::Paused,
        }
    }
}

pub(crate) async fn status(vm: QemuVmConfig, args: StatusArgs) -> Result<(), crate::Error> {
    let stream = QmpStreamTokio::open_uds(vm.get_uds_path(&args.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();
    let status = qmp
        .execute(qapi::qmp::query_status {})
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    let status = VmStatusResponse {
        status: VmStatus::from_qmp(status.status),
        message: match status.status {
            RunState::debug => "Running with a debugger",
            RunState::inmigrate => "Waiting for an incoming migration.",
            RunState::internal_error => {
                "Internal error that prevents further guest execution has occurred."
            }
            RunState::io_error => "I/O error",
            RunState::paused => "Paused because of 'stop' command.",
            RunState::postmigrate => "Paused after successful 'migrate' command.",
            RunState::prelaunch => "Prelaunch",
            RunState::finish_migrate => "Paused to finish the migration process.",
            RunState::restore_vm => "Restoring VM state.",
            RunState::running => "Actively running.",
            RunState::save_vm => "Saving VM state.",
            RunState::shutdown => "Shutdown",
            RunState::suspended => "Suspended (ACPI S3).",
            RunState::watchdog => "Watchdog was triggered.",
            RunState::guest_panicked => "Guest OS panicked.",
            RunState::colo => "save/restore VM state under colo checkpoint.",
        }
        .to_string(),
    };
    serde_json::to_writer(std::io::stdout(), &status)
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    Ok(())
}
