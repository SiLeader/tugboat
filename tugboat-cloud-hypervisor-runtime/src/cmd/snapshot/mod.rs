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
use serde_json::json;
use std::path::PathBuf;
use tracing::warn;
use tugboat_runtime_common::config::load_config;
use tugboat_runtime_common::snapshot::ship_snapshot_dir;
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::snapshot::{
    VmSnapshotCreateRequest, VmSnapshotCreateResponse, VmSnapshotDeleteRequest, VmSnapshotEntry,
    VmSnapshotListRequest, VmSnapshotListResponse, VmSnapshotMode, VmSnapshotRestoreRequest,
};

const RUNTIME_NAME: &str = "cloud-hypervisor";

#[derive(Debug, Parser)]
pub struct SnapshotCreateArgs {
    #[arg(help = "Path to the snapshot create request config file or - for stdin")]
    config: String,
}

#[derive(Debug, Parser)]
pub struct SnapshotDeleteArgs {
    #[arg(help = "Path to the snapshot delete request config file or - for stdin")]
    config: String,
}

#[derive(Debug, Parser)]
pub struct SnapshotRestoreArgs {
    #[arg(help = "Path to the snapshot restore request config file or - for stdin")]
    config: String,
}

#[derive(Debug, Parser)]
pub struct SnapshotListArgs {
    #[arg(help = "Path to the snapshot list request config file or - for stdin")]
    config: String,
}

pub async fn snapshot_create(
    config: CloudHypervisorVmConfig,
    args: SnapshotCreateArgs,
) -> crate::Result<()> {
    let req: VmSnapshotCreateRequest = load_config(args.config)?;
    let response = snapshot_create_request(&config, &req).await?;
    serde_json::to_writer(std::io::stdout(), &response).map_err(crate::Error::Json)?;
    Ok(())
}

async fn snapshot_create_request(
    config: &CloudHypervisorVmConfig,
    req: &VmSnapshotCreateRequest,
) -> crate::Result<VmSnapshotCreateResponse> {
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let handle = derive_snapshot_handle(&req.ship_id);
    let ship_dir = ship_snapshot_dir(&config.snapshot_dir_path(), &req.ship_id)
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    let snapshot_dir = ship_dir.join(&handle);
    std::fs::create_dir_all(&snapshot_dir)?;

    let mut client = ChApiClient::connect(config.get_api_socket_path(&req.ship_id)).await?;

    let resumed = match req.mode {
        VmSnapshotMode::Online => {
            client.put("/api/v1/vm.pause", None).await?;
            true
        }
        VmSnapshotMode::Offline => false,
    };

    let destination_url = format!("file://{}", snapshot_dir.display());
    let snapshot_result = client
        .put(
            "/api/v1/vm.snapshot",
            Some(&json!({ "destination_url": destination_url })),
        )
        .await;

    // Always try to resume the guest in Online mode, even if vm.snapshot failed.
    if resumed && let Err(e) = client.put("/api/v1/vm.resume", None).await {
        warn!("Failed to resume VM after snapshot: {e}");
    }
    snapshot_result?;

    let response = VmSnapshotCreateResponse {
        handle,
        runtime: RUNTIME_NAME.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        size_bytes: directory_size_bytes(&snapshot_dir),
    };
    Ok(response)
}

pub async fn snapshot_delete(
    config: CloudHypervisorVmConfig,
    args: SnapshotDeleteArgs,
) -> crate::Result<()> {
    let req: VmSnapshotDeleteRequest = load_config(args.config)?;
    snapshot_delete_request(&config, &req).await
}

async fn snapshot_delete_request(
    config: &CloudHypervisorVmConfig,
    req: &VmSnapshotDeleteRequest,
) -> crate::Result<()> {
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let ship_dir = ship_snapshot_dir(&config.snapshot_dir_path(), &req.ship_id)
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    let snapshot_dir = ship_dir.join(&req.handle);
    if !snapshot_dir.exists() {
        return Err(crate::Error::ActionFailed(format!(
            "snapshot handle '{}' not found for ship '{}'",
            req.handle, req.ship_id
        )));
    }
    std::fs::remove_dir_all(&snapshot_dir)?;
    Ok(())
}

pub async fn snapshot_restore(
    config: CloudHypervisorVmConfig,
    args: SnapshotRestoreArgs,
) -> crate::Result<()> {
    let req: VmSnapshotRestoreRequest = load_config(args.config)?;
    snapshot_restore_request(&config, &req).await
}

async fn snapshot_restore_request(
    config: &CloudHypervisorVmConfig,
    req: &VmSnapshotRestoreRequest,
) -> crate::Result<()> {
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let ship_dir = ship_snapshot_dir(&config.snapshot_dir_path(), &req.ship_id)
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    let snapshot_dir = ship_dir.join(&req.handle);
    if !snapshot_dir.exists() {
        return Err(crate::Error::ActionFailed(format!(
            "snapshot handle '{}' not found for ship '{}'",
            req.handle, req.ship_id
        )));
    }
    let source_url = format!("file://{}", snapshot_dir.display());

    let mut client = ChApiClient::connect(config.get_api_socket_path(&req.ship_id)).await?;
    client
        .put(
            "/api/v1/vm.restore",
            Some(&json!({ "source_url": source_url })),
        )
        .await?;
    // Restored Cloud Hypervisor VMs come back paused; resume the guest.
    client.put("/api/v1/vm.resume", None).await?;
    Ok(())
}

pub async fn snapshot_list(
    config: CloudHypervisorVmConfig,
    args: SnapshotListArgs,
) -> crate::Result<()> {
    let req: VmSnapshotListRequest = load_config(args.config)?;
    let response = snapshot_list_request(&config, &req)?;
    serde_json::to_writer(std::io::stdout(), &response).map_err(crate::Error::Json)?;
    Ok(())
}

fn snapshot_list_request(
    config: &CloudHypervisorVmConfig,
    req: &VmSnapshotListRequest,
) -> crate::Result<VmSnapshotListResponse> {
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let ship_dir = ship_snapshot_dir(&config.snapshot_dir_path(), &req.ship_id)
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    let mut snapshots = Vec::new();
    if !ship_dir.exists() {
        return Ok(VmSnapshotListResponse { snapshots });
    }
    for entry in std::fs::read_dir(&ship_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let handle = entry.file_name().to_string_lossy().into_owned();
        let metadata = entry.metadata().ok();
        let created_at = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339().into())
            .unwrap_or_default();
        snapshots.push(VmSnapshotEntry {
            handle,
            created_at,
            size_bytes: directory_size_bytes(&entry.path()),
        });
    }
    snapshots.sort_by(|a, b| a.handle.cmp(&b.handle));
    Ok(VmSnapshotListResponse { snapshots })
}

fn derive_snapshot_handle(ship_id: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let suffix = &suffix[..8];
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    format!("{ship_id}-{timestamp}-{suffix}")
}

fn directory_size_bytes(path: &PathBuf) -> Option<u64> {
    let mut total: u64 = 0;
    let entries = std::fs::read_dir(path).ok()?;
    for entry in entries.flatten() {
        let metadata = entry.metadata().ok()?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Some(total)
}

// Always reference the value to silence dead-code warnings when only the
// type is exposed publicly elsewhere.
const _: fn() -> VmSnapshotMode = || VmSnapshotMode::default();

#[cfg(test)]
mod tests {
    use super::{
        derive_snapshot_handle, snapshot_create_request, snapshot_delete_request,
        snapshot_list_request, snapshot_restore_request,
    };
    use crate::testing::{TestVm, http_empty_response, http_json_response, spawn_mock_server};
    use serde_json::json;
    use tugboat_vm_runtime_interface::snapshot::{
        VmSnapshotCreateRequest, VmSnapshotDeleteRequest, VmSnapshotListRequest, VmSnapshotMode,
        VmSnapshotRestoreRequest,
    };

    #[test]
    fn derive_snapshot_handle_includes_ship_id() {
        let handle = derive_snapshot_handle("ship-a");
        assert!(handle.starts_with("ship-a-"));
    }

    #[tokio::test]
    async fn snapshot_create_pauses_snapshots_and_resumes() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-create");
        let snapshot_dir = test_vm
            .runtime_request_path("snapshots")
            .to_string_lossy()
            .into_owned();
        test_vm.config.snapshot_dir = Some(snapshot_dir.clone());

        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_empty_response("204 No Content"),
                http_empty_response("204 No Content"),
                http_empty_response("204 No Content"),
            ],
        );

        let request = VmSnapshotCreateRequest {
            ship_id: "vm-create".into(),
            mode: VmSnapshotMode::Online,
        };
        let response = snapshot_create_request(&test_vm.config, &request)
            .await
            .unwrap();
        assert_eq!(response.runtime, "cloud-hypervisor");
        assert!(response.handle.starts_with("vm-create-"));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/vm.pause");
        assert_eq!(requests[1].path, "/api/v1/vm.snapshot");
        assert_eq!(requests[2].path, "/api/v1/vm.resume");
        let snapshot_body: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        let destination = snapshot_body["destination_url"].as_str().unwrap();
        assert!(destination.starts_with("file:///"));
        assert!(destination.contains(&response.handle));
    }

    #[tokio::test]
    async fn snapshot_create_offline_does_not_pause_or_resume() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-offline");
        test_vm.config.snapshot_dir = Some(
            test_vm
                .runtime_request_path("snapshots")
                .to_string_lossy()
                .into_owned(),
        );

        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_empty_response("204 No Content")],
        );

        let request = VmSnapshotCreateRequest {
            ship_id: "vm-offline".into(),
            mode: VmSnapshotMode::Offline,
        };
        snapshot_create_request(&test_vm.config, &request)
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/api/v1/vm.snapshot");
    }

    #[tokio::test]
    async fn snapshot_restore_calls_restore_and_resume() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-restore");
        let snapshot_dir = test_vm.runtime_request_path("snapshots");
        let handle_dir = snapshot_dir.join("vm-restore").join("snap-a");
        std::fs::create_dir_all(&handle_dir).unwrap();
        test_vm.config.snapshot_dir = Some(snapshot_dir.to_string_lossy().into_owned());

        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_empty_response("204 No Content"),
                http_empty_response("204 No Content"),
            ],
        );

        let request = VmSnapshotRestoreRequest {
            ship_id: "vm-restore".into(),
            handle: "snap-a".into(),
        };
        snapshot_restore_request(&test_vm.config, &request)
            .await
            .unwrap();

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/api/v1/vm.restore");
        assert_eq!(requests[1].path, "/api/v1/vm.resume");
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body["source_url"].as_str().unwrap().contains("snap-a"));
    }

    #[tokio::test]
    async fn snapshot_restore_rejects_missing_handle() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-missing");
        test_vm.config.snapshot_dir = Some(
            test_vm
                .runtime_request_path("snapshots")
                .to_string_lossy()
                .into_owned(),
        );

        let request = VmSnapshotRestoreRequest {
            ship_id: "vm-missing".into(),
            handle: "absent".into(),
        };
        let err = snapshot_restore_request(&test_vm.config, &request)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::Error::ActionFailed(_)));
    }

    #[tokio::test]
    async fn snapshot_delete_removes_directory() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-delete");
        let snapshot_dir = test_vm.runtime_request_path("snapshots");
        let handle_dir = snapshot_dir.join("vm-delete").join("snap-a");
        std::fs::create_dir_all(&handle_dir).unwrap();
        test_vm.config.snapshot_dir = Some(snapshot_dir.to_string_lossy().into_owned());

        let request = VmSnapshotDeleteRequest {
            ship_id: "vm-delete".into(),
            handle: "snap-a".into(),
        };
        snapshot_delete_request(&test_vm.config, &request)
            .await
            .unwrap();
        assert!(!handle_dir.exists());
    }

    #[test]
    fn snapshot_list_enumerates_directories() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-list");
        let snapshot_dir = test_vm.runtime_request_path("snapshots");
        let ship_dir = snapshot_dir.join("vm-list");
        std::fs::create_dir_all(ship_dir.join("snap-a")).unwrap();
        std::fs::create_dir_all(ship_dir.join("snap-b")).unwrap();
        test_vm.config.snapshot_dir = Some(snapshot_dir.to_string_lossy().into_owned());

        let request = VmSnapshotListRequest {
            ship_id: "vm-list".into(),
        };
        let response = snapshot_list_request(&test_vm.config, &request).unwrap();
        let handles: Vec<&str> = response
            .snapshots
            .iter()
            .map(|s| s.handle.as_str())
            .collect();
        assert_eq!(handles, vec!["snap-a", "snap-b"]);
    }

    #[test]
    fn snapshot_list_returns_empty_when_dir_missing() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-empty");
        test_vm.config.snapshot_dir = Some(
            test_vm
                .runtime_request_path("snapshots")
                .to_string_lossy()
                .into_owned(),
        );

        let request = VmSnapshotListRequest {
            ship_id: "vm-empty".into(),
        };
        let response = snapshot_list_request(&test_vm.config, &request).unwrap();
        assert!(response.snapshots.is_empty());
    }

    #[tokio::test]
    async fn snapshot_create_resumes_after_snapshot_failure() {
        let mut test_vm = TestVm::new("tugboat-ch-snapshot", "vm-fail");
        test_vm.config.snapshot_dir = Some(
            test_vm
                .runtime_request_path("snapshots")
                .to_string_lossy()
                .into_owned(),
        );

        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![
                http_empty_response("204 No Content"),
                http_json_response("500 Internal Server Error", json!({"error":"boom"})),
                http_empty_response("204 No Content"),
            ],
        );

        let request = VmSnapshotCreateRequest {
            ship_id: "vm-fail".into(),
            mode: VmSnapshotMode::Online,
        };
        let err = snapshot_create_request(&test_vm.config, &request)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::Error::Api(_)));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/v1/vm.pause");
        assert_eq!(requests[1].path, "/api/v1/vm.snapshot");
        assert_eq!(requests[2].path, "/api/v1/vm.resume");
    }
}
