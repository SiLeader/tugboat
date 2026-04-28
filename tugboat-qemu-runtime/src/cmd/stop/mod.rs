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
use crate::execute::vm::QemuVmConfig;
use clap::Parser;
use tracing::info;
use tugboat_runtime_common::config::load_config;
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::stop::{VmStopRequest, VmStopType};

#[derive(Debug, Parser)]
pub struct StopArgs {
    #[arg(help = "Path to the stop request config file or - for stdin")]
    config: String,
}

pub async fn stop(config: QemuVmConfig, args: StopArgs) -> crate::Result<()> {
    let req: VmStopRequest = load_config(args.config)?;
    validate_safe_id(&req.id, "vm id")?;

    let (qmp, _handle) = connect_qmp(config.get_uds_path(&req.id))
        .await?
        .spawn_tokio();

    match req.stop_type {
        VmStopType::Shutdown => {
            info!("Sending system_powerdown to VM {}", req.id);
            execute_with_timeout(
                qmp.execute(qapi::qmp::system_powerdown {}),
                "Timed out issuing system_powerdown",
            )
            .await?;
        }
        VmStopType::PowerOff => {
            info!("Sending quit to VM {}", req.id);
            execute_with_timeout(qmp.execute(qapi::qmp::quit {}), "Timed out issuing quit").await?;
        }
    }

    Ok(())
}
