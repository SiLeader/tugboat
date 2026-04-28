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
use std::path::Path;
use tracing::info;
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::migrate::{VmMigrationPhase, VmMigrationStatusResponse};

#[derive(Debug, Parser)]
pub struct MigrationStatusArgs {
    #[arg(help = "The VM ID")]
    id: String,
}

pub async fn status(
    config: CloudHypervisorVmConfig,
    args: MigrationStatusArgs,
) -> crate::Result<()> {
    let mut stdout = std::io::stdout();
    status_with_writer(&config, &args.id, &mut stdout).await
}

async fn status_with_writer(
    vm: &CloudHypervisorVmConfig,
    id: &str,
    writer: &mut impl Write,
) -> crate::Result<()> {
    let response = query_status(vm, id).await?;
    write_status_response(writer, &response)
}

async fn query_status(
    vm: &CloudHypervisorVmConfig,
    id: &str,
) -> crate::Result<VmMigrationStatusResponse> {
    validate_safe_id(id, "vm id")?;

    let socket_path = vm.get_api_socket_path(id);
    let mut client = ChApiClient::connect(&socket_path).await?;
    let vm_info: VmInfoResponse = serde_json::from_value(client.get("/api/v1/vm.info").await?)?;
    let response =
        map_cloud_hypervisor_migration_state(&vm_info.state, read_migration_event_state(vm, id)?)?;

    info!(
        "Fetched migration status for VM {id} from Cloud Hypervisor state {}",
        vm_info.state
    );
    Ok(response)
}

fn write_status_response(
    mut writer: impl Write,
    response: &VmMigrationStatusResponse,
) -> crate::Result<()> {
    serde_json::to_writer(&mut writer, response)?;
    Ok(())
}

fn map_cloud_hypervisor_migration_state(
    state: &str,
    shutdown_event_state: ShutdownEventState,
) -> crate::Result<VmMigrationStatusResponse> {
    let (phase, message) = match state {
        "Created" | "Running" => (
            VmMigrationPhase::None,
            "Migration is not active on this VM.",
        ),
        "Paused" | "BreakPoint" => (
            VmMigrationPhase::Active,
            "VM is paused and migration may be in progress.",
        ),
        "Shutdown" => shutdown_event_state.into_response(),
        other => {
            return Err(crate::Error::Api(format!(
                "unknown Cloud Hypervisor VM state: {other}"
            )));
        }
    };

    Ok(VmMigrationStatusResponse {
        phase,
        message: message.to_string(),
        bytes_transferred: None,
        bytes_remaining: None,
        ram_dirty_rate_mbps: None,
    })
}

#[derive(Debug, Deserialize)]
struct VmInfoResponse {
    state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownEventState {
    None,
    Started,
    Failed,
    Finished,
}

impl ShutdownEventState {
    fn into_response(self) -> (VmMigrationPhase, &'static str) {
        match self {
            Self::Finished => (
                VmMigrationPhase::Completed,
                "VM is shut down after Cloud Hypervisor reported migration completion.",
            ),
            Self::Failed => (
                VmMigrationPhase::Failed,
                "VM is shut down after Cloud Hypervisor reported migration failure.",
            ),
            Self::Started => (
                VmMigrationPhase::Failed,
                "VM is shut down and migration started, but completion was never reported.",
            ),
            Self::None => (
                VmMigrationPhase::None,
                "VM is shut down and no migration activity was recorded.",
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct EventRecord {
    source: String,
    event: String,
}

fn read_migration_event_state(
    vm: &CloudHypervisorVmConfig,
    id: &str,
) -> crate::Result<ShutdownEventState> {
    let event_path = vm.get_event_path(id);
    if !Path::new(&event_path).exists() {
        return Ok(ShutdownEventState::None);
    }

    let content = std::fs::read_to_string(&event_path)?;
    if content.trim().is_empty() {
        return Ok(ShutdownEventState::None);
    }

    let mut last_state = ShutdownEventState::None;
    for event in serde_json::Deserializer::from_str(&content).into_iter::<EventRecord>() {
        let event = event?;
        if event.source != "vm" {
            continue;
        }

        last_state = match event.event.as_str() {
            "migration-started" => ShutdownEventState::Started,
            "migration-failed" => ShutdownEventState::Failed,
            "migration-finished" => ShutdownEventState::Finished,
            _ => last_state,
        };
    }

    Ok(last_state)
}

#[cfg(test)]
mod tests {
    use super::{
        ShutdownEventState, map_cloud_hypervisor_migration_state, query_status,
        read_migration_event_state, status_with_writer, write_status_response,
    };
    use crate::testing::{TestVm, http_response, spawn_mock_server};
    use serde_json::json;
    use tugboat_vm_runtime_interface::migrate::VmMigrationPhase;

    #[tokio::test]
    async fn test_migration_status_running() {
        let test_vm = TestVm::new("tugboat-ch-migration-status", "vm-running");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(r#"{"state":"Running"}"#))],
        );

        let response = query_status(&test_vm.config, "vm-running").await.unwrap();
        let requests = server.await.unwrap();

        assert!(matches!(response.phase, VmMigrationPhase::None));
        assert_eq!(response.message, "Migration is not active on this VM.");
        assert_eq!(response.bytes_transferred, None);
        assert_eq!(response.bytes_remaining, None);
        assert_eq!(response.ram_dirty_rate_mbps, None);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/v1/vm.info");
        assert_eq!(
            request.headers.get("content-length").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn test_migration_status_all_states() {
        let cases = [
            (
                "Created",
                VmMigrationPhase::None,
                "Migration is not active on this VM.",
                ShutdownEventState::None,
            ),
            (
                "Running",
                VmMigrationPhase::None,
                "Migration is not active on this VM.",
                ShutdownEventState::None,
            ),
            (
                "Paused",
                VmMigrationPhase::Active,
                "VM is paused and migration may be in progress.",
                ShutdownEventState::None,
            ),
            (
                "BreakPoint",
                VmMigrationPhase::Active,
                "VM is paused and migration may be in progress.",
                ShutdownEventState::None,
            ),
            (
                "Shutdown",
                VmMigrationPhase::Completed,
                "VM is shut down after Cloud Hypervisor reported migration completion.",
                ShutdownEventState::Finished,
            ),
        ];

        for (state, expected_phase, expected_message, shutdown_event_state) in cases {
            let response =
                map_cloud_hypervisor_migration_state(state, shutdown_event_state).unwrap();
            assert!(matches!(
                (response.phase, expected_phase),
                (VmMigrationPhase::None, VmMigrationPhase::None)
                    | (VmMigrationPhase::Active, VmMigrationPhase::Active)
                    | (VmMigrationPhase::Completed, VmMigrationPhase::Completed)
            ));
            assert_eq!(response.message, expected_message);
            assert_eq!(response.bytes_transferred, None);
            assert_eq!(response.bytes_remaining, None);
            assert_eq!(response.ram_dirty_rate_mbps, None);
        }
    }

    #[test]
    fn test_migration_status_response_is_written_as_json() {
        let mut output = Vec::new();
        write_status_response(
            &mut output,
            &tugboat_vm_runtime_interface::migrate::VmMigrationStatusResponse {
                phase: VmMigrationPhase::Active,
                message: "VM is paused and migration may be in progress.".into(),
                bytes_transferred: None,
                bytes_remaining: None,
                ram_dirty_rate_mbps: None,
            },
        )
        .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            value,
            json!({
                "phase": "Active",
                "message": "VM is paused and migration may be in progress.",
            })
        );
    }

    #[test]
    fn shutdown_without_migration_events_is_not_treated_as_completed() {
        let response =
            map_cloud_hypervisor_migration_state("Shutdown", ShutdownEventState::None).unwrap();

        assert!(matches!(response.phase, VmMigrationPhase::None));
        assert_eq!(
            response.message,
            "VM is shut down and no migration activity was recorded."
        );
    }

    #[test]
    fn shutdown_after_failed_migration_is_reported_as_failed() {
        let response =
            map_cloud_hypervisor_migration_state("Shutdown", ShutdownEventState::Failed).unwrap();

        assert!(matches!(response.phase, VmMigrationPhase::Failed));
        assert_eq!(
            response.message,
            "VM is shut down after Cloud Hypervisor reported migration failure."
        );
    }

    #[test]
    fn reads_last_migration_event_from_event_monitor_log() {
        let test_vm = TestVm::new("tugboat-ch-migration-status", "vm-events");
        std::fs::write(
            test_vm.config.get_event_path("vm-events"),
            r#"{
  "timestamp": {"secs": 0, "nanos": 0},
  "source": "vm",
  "event": "migration-started",
  "properties": null
}

{
  "timestamp": {"secs": 1, "nanos": 0},
  "source": "vm",
  "event": "migration-finished",
  "properties": null
}
"#,
        )
        .unwrap();

        let state = read_migration_event_state(&test_vm.config, "vm-events").unwrap();
        assert_eq!(state, ShutdownEventState::Finished);
    }

    #[tokio::test]
    async fn test_migration_status_entrypoint_uses_vm_id_argument() {
        let test_vm = TestVm::new("tugboat-ch-migration-status", "vm-entrypoint");
        std::fs::write(
            test_vm.config.get_event_path("vm-entrypoint"),
            r#"{
  "timestamp": {"secs": 0, "nanos": 0},
  "source": "vm",
  "event": "migration-finished",
  "properties": null
}
"#,
        )
        .unwrap();
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(r#"{"state":"Shutdown"}"#))],
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
                "phase": "Completed",
                "message": "VM is shut down after Cloud Hypervisor reported migration completion.",
            })
        );
    }
}
