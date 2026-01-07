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

mod create;
pub(crate) mod error;
mod inner;
mod start;
mod status;

use crate::runtime::error::RuntimeError;
use crate::runtime::inner::Runtime;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tracing::error;
use tugboat_vm_image::VmImageRegistry;

#[derive(Clone)]
pub(crate) struct RuntimeOperator {
    config: RuntimeConfig,
    registry: VmImageRegistry,
    children: Arc<RwLock<HashMap<String, Runtime>>>,
}

impl RuntimeOperator {
    pub(crate) fn new(config: RuntimeConfig, image_dir: impl AsRef<Path>) -> Self {
        Self {
            config,
            registry: VmImageRegistry::new(image_dir.as_ref().to_path_buf()),
            children: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn run_command(
        &self,
        subcommand: &str,
        args: &impl Serialize,
    ) -> Result<Child, RuntimeError> {
        let vm_config = serde_json::to_string(args)?;
        let mut child = self
            .config
            .runtime_command()
            .args([subcommand, "-"])
            .stdin(Stdio::piped())
            .spawn()?;
        match &mut child.stdin {
            Some(stdin) => {
                if let Err(e) = stdin.write_all(vm_config.as_bytes()).await {
                    error!("Cannot write config: {e}");
                    kill_impl(child).await?;
                    Err(RuntimeError::Io(e))
                } else {
                    Ok(child)
                }
            }
            None => {
                kill_impl(child).await?;
                Err(RuntimeError::RunVm)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeConfig {
    pub executable: String,
    pub args: Vec<String>,
}

impl RuntimeConfig {
    fn runtime_command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.args);
        command
    }
}

async fn kill_impl(mut child: Child) -> Result<(), RuntimeError> {
    child.kill().await?;
    tokio::spawn(async move {
        if let Err(e) = child.wait().await {
            error!("Cannot wait child: {e}");
        }
    });
    Ok(())
}

async fn handle_command_response(child: Child) -> Result<(), RuntimeError> {
    let output = child.wait_with_output().await?;

    if output.status.success() {
        Ok(())
    } else {
        Err(RuntimeError::CommandFailed(
            output.status,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}
