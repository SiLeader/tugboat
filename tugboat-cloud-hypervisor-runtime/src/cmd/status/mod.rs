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
use serde::Deserialize;
use std::io::Write;
use tracing::info;
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::status::{VmStatus, VmStatusResponse};

#[derive(Debug, Parser)]
pub(crate) struct StatusArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

pub(crate) async fn status(
    vm: CloudHypervisorVmConfig,
    args: StatusArgs,
) -> Result<(), crate::Error> {
    let mut stdout = std::io::stdout();
    status_with_writer(&vm, &args.id, &mut stdout).await
}

async fn status_with_writer(
    vm: &CloudHypervisorVmConfig,
    id: &str,
    writer: &mut impl Write,
) -> Result<(), crate::Error> {
    let response = query_status(vm, id).await?;
    write_status_response(writer, &response)
}

async fn query_status(
    vm: &CloudHypervisorVmConfig,
    id: &str,
) -> Result<VmStatusResponse, crate::Error> {
    validate_safe_id(id, "vm id")?;

    let socket_path = vm.get_api_socket_path(id);
    let mut client = ChApiClient::connect(&socket_path).await?;
    let vm_info: VmInfoResponse = serde_json::from_value(client.get("/api/v1/vm.info").await?)?;
    let (status, message) = map_cloud_hypervisor_state(&vm_info.state)?;

    info!("Fetched status for VM {id}: {}", vm_info.state);

    Ok(VmStatusResponse { status, message })
}

fn write_status_response(
    mut writer: impl Write,
    response: &VmStatusResponse,
) -> Result<(), crate::Error> {
    serde_json::to_writer(&mut writer, response)?;
    Ok(())
}

fn map_cloud_hypervisor_state(state: &str) -> Result<(VmStatus, String), crate::Error> {
    let mapped = match state {
        "Created" => (
            VmStatus::Prelaunch,
            "VM has been created but not started yet.",
        ),
        "Running" => (VmStatus::Running, "VM is actively running."),
        "Shutdown" => (VmStatus::Shutdown, "VM has been shut down."),
        "Paused" => (VmStatus::Paused, "VM is paused."),
        "BreakPoint" => (VmStatus::Paused, "VM is paused at a breakpoint."),
        other => {
            return Err(crate::Error::Api(format!(
                "unknown Cloud Hypervisor VM state: {other}"
            )));
        }
    };

    Ok((mapped.0, mapped.1.to_string()))
}

#[derive(Debug, Deserialize)]
struct VmInfoResponse {
    state: String,
}

#[cfg(test)]
mod tests {
    use super::{
        map_cloud_hypervisor_state, query_status, status_with_writer, write_status_response,
    };
    use crate::testing::{TestVm, http_response, spawn_mock_server};
    use serde_json::json;
    use tugboat_vm_runtime_interface::status::VmStatus;

    #[tokio::test]
    async fn test_status_running() {
        let test_vm = TestVm::new("tugboat-ch-status", "vm-running");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(r#"{"state":"Running"}"#))],
        );

        let response = query_status(&test_vm.config, "vm-running").await.unwrap();
        let requests = server.await.unwrap();

        assert!(matches!(response.status, VmStatus::Running));
        assert_eq!(response.message, "VM is actively running.");
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/v1/vm.info");
        assert_eq!(
            request.headers.get("content-length").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn test_status_all_states() {
        let cases = [
            (
                "Created",
                VmStatus::Prelaunch,
                "VM has been created but not started yet.",
            ),
            ("Running", VmStatus::Running, "VM is actively running."),
            ("Shutdown", VmStatus::Shutdown, "VM has been shut down."),
            ("Paused", VmStatus::Paused, "VM is paused."),
            (
                "BreakPoint",
                VmStatus::Paused,
                "VM is paused at a breakpoint.",
            ),
        ];

        for (state, expected_status, expected_message) in cases {
            let (status, message) = map_cloud_hypervisor_state(state).unwrap();
            assert!(matches!(
                (status, expected_status),
                (VmStatus::Prelaunch, VmStatus::Prelaunch)
                    | (VmStatus::Running, VmStatus::Running)
                    | (VmStatus::Shutdown, VmStatus::Shutdown)
                    | (VmStatus::Paused, VmStatus::Paused)
            ));
            assert_eq!(message, expected_message);
        }
    }

    #[test]
    fn test_status_response_is_written_as_json() {
        let mut output = Vec::new();
        write_status_response(
            &mut output,
            &tugboat_vm_runtime_interface::status::VmStatusResponse {
                status: VmStatus::Running,
                message: "VM is actively running.".into(),
            },
        )
        .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            value,
            json!({
                "status": "Running",
                "message": "VM is actively running.",
            })
        );
    }

    #[tokio::test]
    async fn test_status_command_writes_stdout_json() {
        let test_vm = TestVm::new("tugboat-ch-status", "vm-stdout");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(r#"{"state":"Shutdown"}"#))],
        );

        let response = query_status(&test_vm.config, "vm-stdout").await.unwrap();
        let mut output = Vec::new();
        write_status_response(&mut output, &response).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();

        assert_eq!(
            value,
            json!({
                "status": "Shutdown",
                "message": "VM has been shut down.",
            })
        );

        let requests = server.await.unwrap();
        assert_eq!(requests[0].path, "/api/v1/vm.info");
    }

    #[tokio::test]
    async fn test_status_entrypoint_uses_vm_id_argument() {
        let test_vm = TestVm::new("tugboat-ch-status", "vm-entrypoint");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(r#"{"state":"Created"}"#))],
        );
        let mut output = Vec::new();

        status_with_writer(&test_vm.config, "vm-entrypoint", &mut output)
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests[0].path, "/api/v1/vm.info");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap(),
            json!({
                "status": "Prelaunch",
                "message": "VM has been created but not started yet.",
            })
        );
    }
}
