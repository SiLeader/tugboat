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
use tracing::info;
use tugboat_vm_runtime_interface::stop::{VmStopRequest, VmStopType};

#[derive(Debug, Parser)]
pub struct StopArgs {
    #[arg(help = "Path to the stop request config file or - for stdin")]
    config: String,
}

pub async fn stop(config: QemuVmConfig, args: StopArgs) -> crate::Result<()> {
    let req: VmStopRequest = load_config_or_panic(args.config);

    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&req.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();

    match req.stop_type {
        VmStopType::Shutdown => {
            info!("Sending system_powerdown to VM {}", req.id);
            qmp.execute(qapi::qmp::system_powerdown {})
                .await
                .map_err(|e| crate::Error::Qmp(e.to_string()))?;
        }
        VmStopType::PowerOff => {
            info!("Sending quit to VM {}", req.id);
            qmp.execute(qapi::qmp::quit {})
                .await
                .map_err(|e| crate::Error::Qmp(e.to_string()))?;
        }
    }

    Ok(())
}
