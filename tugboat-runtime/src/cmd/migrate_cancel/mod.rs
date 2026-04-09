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

#[derive(Debug, Parser)]
pub struct MigrateCancelArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

pub async fn migrate_cancel(config: QemuVmConfig, args: MigrateCancelArgs) -> crate::Result<()> {
    crate::validate::validate_safe_id(&args.id, "vm id")?;
    let stream = QmpStreamTokio::open_uds(config.get_uds_path(&args.id))
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let stream = stream
        .negotiate()
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;
    let (qmp, _handle) = stream.spawn_tokio();

    qmp.execute(qapi::qmp::migrate_cancel {})
        .await
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    Ok(())
}
