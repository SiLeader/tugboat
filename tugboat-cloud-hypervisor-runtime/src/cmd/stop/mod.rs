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
use crate::cmd::api::ChApiClient;
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

pub async fn stop(config: CloudHypervisorVmConfig, args: StopArgs) -> crate::Result<()> {
    let req: VmStopRequest = load_config(args.config)?;
    stop_request(&config, &req).await
}

async fn stop_request(config: &CloudHypervisorVmConfig, req: &VmStopRequest) -> crate::Result<()> {
    validate_safe_id(&req.id, "vm id")?;

    let socket_path = config.get_api_socket_path(&req.id);
    let mut client = ChApiClient::connect(&socket_path).await?;

    match req.stop_type {
        VmStopType::Shutdown => {
            info!("Sending ACPI power button event to VM {}", req.id);
            client.put("/api/v1/vm.power-button", None).await?;
        }
        VmStopType::PowerOff => {
            info!("Sending forced shutdown request to VM {}", req.id);
            client.put("/api/v1/vm.shutdown", None).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{StopArgs, stop, stop_request};
    use crate::testing::{TestVm, http_response, spawn_mock_server};
    use serde_json::json;
    use tugboat_vm_runtime_interface::stop::{VmStopRequest, VmStopType};

    #[tokio::test]
    async fn test_stop_shutdown_calls_power_button() {
        let test_vm = TestVm::new("tugboat-ch-stop", "vm-01");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("204 No Content", None)],
        );

        stop_request(
            &test_vm.config,
            &VmStopRequest {
                id: "vm-01".into(),
                stop_type: VmStopType::Shutdown,
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        let request = &requests[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.path, "/api/v1/vm.power-button");
        assert_eq!(
            request.headers.get("content-length").map(String::as_str),
            Some("0")
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn test_stop_poweroff_calls_shutdown() {
        let test_vm = TestVm::new("tugboat-ch-stop", "vm-02");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("204 No Content", None)],
        );

        stop_request(
            &test_vm.config,
            &VmStopRequest {
                id: "vm-02".into(),
                stop_type: VmStopType::PowerOff,
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        let request = &requests[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.path, "/api/v1/vm.shutdown");
        assert_eq!(
            request.headers.get("content-length").map(String::as_str),
            Some("0")
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn test_stop_reads_request_from_json_file() {
        let test_vm = TestVm::new("tugboat-ch-stop", "vm-03");
        let config_path = test_vm.runtime_request_path("stop.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "id": "vm-03",
                "stopType": "Shutdown",
            }))
            .unwrap(),
        )
        .unwrap();
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("204 No Content", None)],
        );

        stop(
            test_vm.config.clone(),
            StopArgs {
                config: config_path.to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests[0].path, "/api/v1/vm.power-button");
    }
}
