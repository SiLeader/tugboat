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
use tracing::{info, warn};
use tugboat_runtime_common::config::load_config;
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::migrate::VmMigrateRequest;

#[derive(Debug, Parser)]
pub struct MigrateArgs {
    #[arg(help = "Path to the migrate request config file or - for stdin")]
    config: String,
}

pub async fn migrate(config: CloudHypervisorVmConfig, args: MigrateArgs) -> crate::Result<()> {
    let req: VmMigrateRequest = load_config(args.config)?;
    migrate_request(&config, &req).await
}

async fn migrate_request(
    config: &CloudHypervisorVmConfig,
    req: &VmMigrateRequest,
) -> crate::Result<()> {
    validate_safe_id(&req.id, "vm id")?;
    warn_unsupported_migration_options(req);

    let socket_path = config.get_api_socket_path(&req.id);
    let mut client = ChApiClient::connect(&socket_path).await?;
    let destination_url = format!("tcp:{}:{}", req.destination_address, req.destination_port);

    info!(
        "Starting live migration for VM {} to {}",
        req.id, destination_url
    );
    client
        .put(
            "/api/v1/vm.send-migration",
            Some(&json!({
                "destination_url": destination_url,
                "local": false,
            })),
        )
        .await?;
    Ok(())
}

fn warn_unsupported_migration_options(req: &VmMigrateRequest) {
    if let Some(max_bandwidth) = req.max_bandwidth_bytes_per_sec {
        warn!(
            "Ignoring unsupported Cloud Hypervisor migration option max_bandwidth_bytes_per_sec={max_bandwidth}"
        );
    }
    if let Some(downtime_limit_ms) = req.downtime_limit_ms {
        warn!(
            "Ignoring unsupported Cloud Hypervisor migration option downtime_limit_ms={downtime_limit_ms}"
        );
    }
    if let Some(xbzrle_cache_size_bytes) = req.xbzrle_cache_size_bytes {
        warn!(
            "Ignoring unsupported Cloud Hypervisor migration option xbzrle_cache_size_bytes={xbzrle_cache_size_bytes}"
        );
    }
    if req.postcopy_enabled {
        warn!("Ignoring unsupported Cloud Hypervisor migration option postcopy_enabled=true");
    }
}

#[cfg(test)]
mod tests {
    use super::migrate_request;
    use crate::testing::{TestVm, http_response, spawn_mock_server};
    use serde_json::json;

    #[tokio::test]
    async fn test_migrate_sends_correct_url() {
        let test_vm = TestVm::new("tugboat-ch-migrate", "vm-migrate");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("204 No Content", None)],
        );

        let request = tugboat_vm_runtime_interface::migrate::VmMigrateRequest {
            id: "vm-migrate".into(),
            destination_address: "192.0.2.10".into(),
            destination_port: 4321,
            max_bandwidth_bytes_per_sec: Some(1),
            downtime_limit_ms: Some(2),
            xbzrle_cache_size_bytes: Some(3),
            postcopy_enabled: true,
        };

        migrate_request(&test_vm.config, &request).await.unwrap();

        let requests = server.await.unwrap();
        let captured = &requests[0];
        assert_eq!(captured.method, "PUT");
        assert_eq!(captured.path, "/api/v1/vm.send-migration");
        assert_eq!(
            captured.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&captured.body).unwrap(),
            json!({
                "destination_url": "tcp:192.0.2.10:4321",
                "local": false,
            })
        );
    }

    #[tokio::test]
    async fn test_migrate_rejects_invalid_vm_id() {
        let test_vm = TestVm::new("tugboat-ch-migrate", "vm-invalid");
        let request = tugboat_vm_runtime_interface::migrate::VmMigrateRequest {
            id: "../bad".into(),
            destination_address: "192.0.2.10".into(),
            destination_port: 4321,
            max_bandwidth_bytes_per_sec: None,
            downtime_limit_ms: None,
            xbzrle_cache_size_bytes: None,
            postcopy_enabled: false,
        };

        let error = migrate_request(&test_vm.config, &request)
            .await
            .unwrap_err();
        assert!(matches!(error, crate::Error::Validation(_)));
    }
}
