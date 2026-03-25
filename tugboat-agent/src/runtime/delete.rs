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

use crate::csi::PublishedVolume;
use crate::runtime::RuntimeOperator;
use crate::runtime::error::RuntimeError;
use tokio::time::{Duration, sleep};
use tracing::{debug, info};
use tugboat_vm_runtime_interface::operator::Error as VmRuntimeOperatorError;
use tugboat_vm_runtime_interface::status::VmStatus;
use tugboat_vm_runtime_interface::stop::{VmStopRequest, VmStopType};

const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SHUTDOWN_POLL_ATTEMPTS: usize = 30;

impl RuntimeOperator {
    pub(crate) async fn is_present(&self, id: &str) -> Result<bool, RuntimeError> {
        match self.operator.status(id).await {
            Ok(_) => Ok(true),
            Err(err) if runtime_is_absent(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    pub(crate) async fn delete(&self, id: String) -> Result<Vec<PublishedVolume>, RuntimeError> {
        debug!("Delete VM: {}", id);
        let req = VmStopRequest {
            id: id.clone(),
            stop_type: VmStopType::Shutdown,
        };

        match self.operator.stop(req).await {
            Ok(_) => info!("Shutdown requested for VM '{}'", id),
            Err(err) if runtime_is_absent(&err) => {
                info!("VM '{}' is already absent, continuing cleanup", id)
            }
            Err(e) => return Err(e.into()),
        }
        self.wait_for_stopped(&id).await?;

        let mut children = self.children.write().await;
        let published_volumes = children
            .remove(&id)
            .map(|runtime| runtime.into_published_volumes())
            .unwrap_or_default();

        Ok(published_volumes)
    }

    async fn wait_for_stopped(&self, id: &str) -> Result<(), RuntimeError> {
        for attempt in 0..SHUTDOWN_POLL_ATTEMPTS {
            match self.operator.status(id).await {
                Ok(status) if matches!(status.status, VmStatus::Shutdown) => {
                    info!("VM '{}' reached shutdown state", id);
                    return Ok(());
                }
                Ok(status) => {
                    debug!(
                        "VM '{}' is still in state {:?} while waiting for shutdown (attempt {}/{})",
                        id,
                        status.status,
                        attempt + 1,
                        SHUTDOWN_POLL_ATTEMPTS
                    );
                }
                Err(err) if runtime_is_absent(&err) => {
                    info!(
                        "VM '{}' runtime socket disappeared, assuming it is stopped",
                        id
                    );
                    return Ok(());
                }
                Err(err) => return Err(err.into()),
            }
            sleep(SHUTDOWN_POLL_INTERVAL).await;
        }

        Err(RuntimeError::ShutdownTimeout(id.to_string()))
    }
}

fn runtime_is_absent(err: &VmRuntimeOperatorError) -> bool {
    match err {
        VmRuntimeOperatorError::CommandFailed(_, stdout, stderr) => {
            let combined = format!("{stdout}\n{stderr}");
            combined.contains("Cannot open UDS") || combined.contains("No such file or directory")
        }
        _ => false,
    }
}
