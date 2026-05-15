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

use crate::hotplug::VmHotplugRequest;
use crate::migrate::{VmMigrateCancelRequest, VmMigrateRequest, VmMigrationStatusResponse};
use crate::run::VmRunRequest;
use crate::snapshot::{
    VmSnapshotCreateRequest, VmSnapshotCreateResponse, VmSnapshotDeleteRequest,
    VmSnapshotListRequest, VmSnapshotListResponse, VmSnapshotRestoreRequest,
};
use crate::status::VmStatusResponse;
use crate::stop::VmStopRequest;
use serde::Serialize;
use std::process::{ExitStatus, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::time::{Duration, timeout};
use tracing::{debug, error};

const COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct VmRuntimeOperator {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Stdin is not loaded")]
    Stdin,
    #[error("Failed to execute command: status: {0}, stdout: '{1}', stderr: '{2}'")]
    CommandFailed(ExitStatus, String, String),
    #[error("Pid is missing")]
    PidMissing,
    #[error("Timed out waiting for runtime command response")]
    Timeout,
}

impl VmRuntimeOperator {
    pub fn new(executable: String, args: Vec<String>) -> Self {
        Self { executable, args }
    }

    fn run_command(&self) -> Command {
        let mut c = Command::new(&self.executable);
        c.args(self.args.as_slice());
        c
    }

    async fn call<A>(&self, op: &str, args: &A) -> Result<Child, Error>
    where
        A: Serialize,
    {
        let vm_config = serde_json::to_string(args)?;
        debug!("Calling VM Runtime: {op}({vm_config})");
        let mut command = self.run_command();
        command.kill_on_drop(true);
        let mut child = command.args([op, "-"]).stdin(Stdio::piped()).spawn()?;
        match &mut child.stdin {
            Some(stdin) => {
                debug!("Writing config to stdin: {vm_config}");
                if let Err(e) = stdin.write_all(vm_config.as_bytes()).await {
                    error!("Cannot write config: {e}");
                    kill_impl(child).await?;
                    Err(Error::Io(e))
                } else {
                    Ok(child)
                }
            }
            None => {
                debug!("Stdin is not loaded");
                kill_impl(child).await?;
                Err(Error::Stdin)
            }
        }
    }

    pub async fn create(&self, args: VmRunRequest) -> Result<u32, Error> {
        let child = self.call("create", &args).await?;
        let pid = match child.id() {
            Some(p) => p,
            None => {
                kill_impl(child).await?;
                return Err(Error::PidMissing);
            }
        };
        handle_command_response(child).await?;
        Ok(pid)
    }

    pub async fn start(&self, id: &str) -> Result<(), Error> {
        let output = self.run_command().args(["start", id]).output().await?;
        handle_output(output)
    }

    pub async fn status(&self, id: &str) -> Result<VmStatusResponse, Error> {
        let output = self.run_command().args(["status", id]).output().await?;

        if output.status.success() {
            let status: VmStatusResponse = serde_json::from_slice(&output.stdout)?;
            Ok(status)
        } else {
            Err(Error::CommandFailed(
                output.status,
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    pub async fn stop(&self, args: VmStopRequest) -> Result<(), Error> {
        let child = self.call("stop", &args).await?;
        handle_command_response(child).await?;
        Ok(())
    }

    pub async fn migrate(&self, args: VmMigrateRequest) -> Result<(), Error> {
        let child = self.call("migrate", &args).await?;
        handle_command_response(child).await?;
        Ok(())
    }

    pub async fn hotplug(&self, args: VmHotplugRequest) -> Result<(), Error> {
        let child = self.call("hotplug", &args).await?;
        handle_command_response(child).await?;
        Ok(())
    }

    pub async fn migrate_cancel(&self, args: VmMigrateCancelRequest) -> Result<(), Error> {
        let output = self
            .run_command()
            .args(["migrate-cancel", &args.id])
            .output()
            .await?;
        handle_output(output)
    }

    pub async fn snapshot_create(
        &self,
        args: VmSnapshotCreateRequest,
    ) -> Result<VmSnapshotCreateResponse, Error> {
        let child = self.call("snapshot-create", &args).await?;
        let output = timeout(COMMAND_RESPONSE_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| Error::Timeout)??;
        if output.status.success() {
            Ok(serde_json::from_slice(&output.stdout)?)
        } else {
            Err(Error::CommandFailed(
                output.status,
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    pub async fn snapshot_delete(&self, args: VmSnapshotDeleteRequest) -> Result<(), Error> {
        let child = self.call("snapshot-delete", &args).await?;
        handle_command_response(child).await
    }

    pub async fn snapshot_restore(&self, args: VmSnapshotRestoreRequest) -> Result<(), Error> {
        let child = self.call("snapshot-restore", &args).await?;
        handle_command_response(child).await
    }

    pub async fn snapshot_list(
        &self,
        args: VmSnapshotListRequest,
    ) -> Result<VmSnapshotListResponse, Error> {
        let child = self.call("snapshot-list", &args).await?;
        let output = timeout(COMMAND_RESPONSE_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| Error::Timeout)??;
        if output.status.success() {
            Ok(serde_json::from_slice(&output.stdout)?)
        } else {
            Err(Error::CommandFailed(
                output.status,
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    pub async fn migration_status(&self, id: &str) -> Result<VmMigrationStatusResponse, Error> {
        let output = self
            .run_command()
            .args(["migration-status", id])
            .output()
            .await?;

        if output.status.success() {
            let status: VmMigrationStatusResponse = serde_json::from_slice(&output.stdout)?;
            Ok(status)
        } else {
            Err(Error::CommandFailed(
                output.status,
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}

async fn kill_impl(mut child: Child) -> Result<(), Error> {
    debug!("Killing child: pid: {}", child.id().unwrap_or_default());
    child.kill().await?;
    debug!("Waiting child: pid: {}", child.id().unwrap_or_default());
    if let Err(e) = child.wait().await {
        error!("Cannot wait child: {e}");
    }
    Ok(())
}

async fn handle_command_response(child: Child) -> Result<(), Error> {
    let output = timeout(COMMAND_RESPONSE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| Error::Timeout)??;
    handle_output(output)
}

fn handle_output(output: std::process::Output) -> Result<(), Error> {
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed(
            output.status,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}
