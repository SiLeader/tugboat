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

use crate::CloudHypervisorVmConfig;
use clap::Parser;
use tugboat_runtime_common::validate::validate_safe_id;

#[derive(Debug, Parser)]
pub struct MigrateCancelArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

pub async fn migrate_cancel(
    _config: CloudHypervisorVmConfig,
    args: MigrateCancelArgs,
) -> crate::Result<()> {
    validate_safe_id(&args.id, "vm id")?;
    Err(crate::Error::ActionFailed(
        "Cloud Hypervisor does not support migration cancellation".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{MigrateCancelArgs, migrate_cancel};
    use crate::{CloudHypervisorBootConfig, CloudHypervisorVmConfig, Error};

    #[tokio::test]
    async fn test_migrate_cancel_returns_error() {
        let error = migrate_cancel(test_config(), MigrateCancelArgs { id: "vm-01".into() })
            .await
            .unwrap_err();

        match error {
            Error::ActionFailed(message) => {
                assert_eq!(
                    message,
                    "Cloud Hypervisor does not support migration cancellation"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn test_config() -> CloudHypervisorVmConfig {
        CloudHypervisorVmConfig {
            executable: "/usr/bin/cloud-hypervisor".into(),
            disk_image_location: "/tmp".into(),
            boot: CloudHypervisorBootConfig {
                kernel: None,
                initramfs: None,
                firmware: None,
            },
            snapshot_dir: None,
        }
    }
}
