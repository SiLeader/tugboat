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
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

#[derive(Debug, Clone)]
pub(crate) struct CniCaller {
    bin_path: PathBuf,
    net_ns_base_path: PathBuf,
}

impl CniCaller {
    const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

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
        cni_type: &str,
        config_file: impl AsRef<Path>,
    ) -> Result<(), crate::error::Error> {
        let file = std::fs::File::open(config_file)?;
        println!("===== BEGIN DUMP NETNS DIR =====");
        if let Ok(rd) = self.net_ns_base_path.read_dir() {
            for entry in rd.flatten() {
                println!("Netns: {:?}", entry.path());
            }
        }
        println!("===== END DUMP NETNS DIR =====");

        let mut child = Command::new(self.bin_path.join(cni_type))
            .env("CNI_COMMAND", command)
            .env("CNI_CONTAINERID", id)
            .env("CNI_NETNS", self.net_ns_base_path.join(id))
            .env("CNI_IFNAME", iface_name)
            .env("CNI_PATH", &self.bin_path)
            .stdin(file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| crate::Error::InvalidConfiguration("stdout is not piped".to_string()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| crate::Error::InvalidConfiguration("stderr is not piped".to_string()))?;

        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes).await;
            bytes
        });
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes).await;
            bytes
        });

        let status = match timeout(Self::COMMAND_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => return Err(crate::Error::Io(e)),
            Err(_) => {
                let _ = child.kill().await;
                return Err(crate::Error::CommandTimeout(format!(
                    "{} {}",
                    cni_type, command
                )));
            }
        };

        let stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();

        if status.success() {
            Ok(())
        } else {
            Err(crate::Error::CommandFailed(
                status,
                String::from_utf8_lossy(&stdout).to_string(),
                String::from_utf8_lossy(&stderr).to_string(),
            ))
        }
    }

    pub(crate) async fn add(
        &self,
        id: &str,
        iface_name: &str,
        cni_type: &str,
        config_file: impl AsRef<Path>,
    ) -> Result<(), crate::error::Error> {
        self.call("ADD", id, iface_name, cni_type, config_file)
            .await
    }

    pub(crate) async fn del(
        &self,
        id: &str,
        iface_name: &str,
        cni_type: &str,
        config_file: impl AsRef<Path>,
    ) -> Result<(), crate::error::Error> {
        self.call("DEL", id, iface_name, cni_type, config_file)
            .await
    }
}
