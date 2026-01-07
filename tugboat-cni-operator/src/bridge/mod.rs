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

use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub(crate) struct BridgeCaller {
    bin_path: PathBuf,
    net_ns_base_path: PathBuf,
}

impl BridgeCaller {
    pub(crate) fn new(bin_path: impl AsRef<Path>, net_ns_base_path: impl AsRef<Path>) -> Self {
        Self {
            bin_path: bin_path.as_ref().to_path_buf(),
            net_ns_base_path: net_ns_base_path.as_ref().to_path_buf(),
        }
    }

    async fn call(
        &self,
        command: &str,
        id: &str,
        iface_name: &str,
        config_file: impl AsRef<Path>,
    ) -> Result<(), crate::error::Error> {
        let file = std::fs::File::open(config_file)?;

        let mut child = Command::new(self.bin_path.join("bridge"))
            .env("CNI_COMMAND", command)
            .env("CNI_CONTAINERID", id)
            .env("CNI_NETNS", self.net_ns_base_path.join(id))
            .env("CNI_IFNAME", iface_name)
            .env("CNI_PATH", &self.bin_path)
            .stdin(file)
            .spawn()?;

        let output = child.wait_with_output().await?;
        if output.status.success() {
            Ok(())
        } else {
            Err(crate::Error::CommandFailed(
                output.status,
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    pub(crate) async fn add(
        &self,
        id: &str,
        iface_name: &str,
        config_file: impl AsRef<Path>,
    ) -> Result<(), crate::error::Error> {
        self.call("ADD", id, iface_name, config_file).await
    }

    pub(crate) async fn del(
        &self,
        id: &str,
        iface_name: &str,
        config_file: impl AsRef<Path>,
    ) -> Result<(), crate::error::Error> {
        self.call("DEL", id, iface_name, config_file).await
    }
}
