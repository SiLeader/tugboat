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

use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Node, Ship, ShipCondition};
use tugboat_resources::manifests::meta::v1::Time;

use super::super::upsert_ship_condition;
use super::super::{PHASE_FAILED, PHASE_READY};

impl ShipReconciler {
    pub(super) async fn local_node_address(&self) -> Result<String, ReconcileError> {
        let api: Api<Node> = Api::all(self.client.clone());
        let Some(node) = api.get(&self.node_name).await? else {
            return Err(ReconcileError::FieldMissing(
                "v1.Node".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let Some(spec) = node.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Node".to_string(),
                "spec".to_string(),
            ));
        };

        // Prefer non-loopback IPv4 addresses.
        for ip in &spec.ips {
            if let Ok(addr) = ip.parse::<std::net::IpAddr>()
                && !addr.is_loopback()
                && addr.is_ipv4()
            {
                return Ok(ip.clone());
            }
        }

        spec.ips.into_iter().next().ok_or_else(|| {
            ReconcileError::FieldMissing("v1.Node".to_string(), "spec.ips[0]".to_string())
        })
    }

    pub(super) async fn find_available_port(&self) -> Result<u16, ReconcileError> {
        // Let the OS assign a free port by binding to port 0.
        // There is an inherent TOCTOU window between dropping this listener and
        // QEMU binding the port, but the gap is sub-millisecond on a dedicated
        // node and is acceptable for the low-frequency migration path.
        let listener = std::net::TcpListener::bind("0.0.0.0:0")
            .map_err(|e| ReconcileError::Runtime(crate::runtime::error::RuntimeError::Io(e)))?;
        let port = listener
            .local_addr()
            .map_err(|e| ReconcileError::Runtime(crate::runtime::error::RuntimeError::Io(e)))?
            .port();
        Ok(port)
    }

    pub(super) async fn mark_migration_target_ready(
        &self,
        ship: &Ship,
        port: u16,
    ) -> Result<(), ReconcileError> {
        let Some(name) = ship.name() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship.namespace().unwrap_or("default");
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let target_address = self.local_node_address().await?;
        let mut conditions = api
            .get(name)
            .await?
            .and_then(|ship| ship.status)
            .map(|status| status.conditions)
            .unwrap_or_default();
        upsert_ship_condition(
            &mut conditions,
            ShipCondition {
                status: "VmMigrationTargetReady".to_string(),
                message: format!(
                    "VM is listening for incoming migration on {target_address}:{port} with deterministic NIC names and MAC addresses"
                ),
                timestamp: Some(Time::now()),
            },
        );

        let patch = serde_json::json!({
            "status": {
                "migration": {
                    "phase": PHASE_READY,
                    "sourceNodeName": ship.spec.as_ref().and_then(|spec| spec.node_name.clone()),
                    "targetNodeName": self.node_name,
                    "targetAddress": target_address,
                    "targetPort": port,
                    "message": "Target VM is ready to accept incoming migration with the same guest NIC identity",
                    "timestamp": Time::now(),
                },
                "conditions": conditions
            }
        });

        api.patch_status(name, patch).await?;
        Ok(())
    }

    pub(super) async fn mark_migration_target_failed(
        &self,
        ship: &Ship,
        message: String,
    ) -> Result<(), ReconcileError> {
        let Some(name) = ship.name() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship.namespace().unwrap_or("default");
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let condition_message = message.clone();
        let mut conditions = api
            .get(name)
            .await?
            .and_then(|ship| ship.status)
            .map(|status| status.conditions)
            .unwrap_or_default();
        upsert_ship_condition(
            &mut conditions,
            ShipCondition {
                status: "VmMigrationFailed".to_string(),
                message: condition_message.clone(),
                timestamp: Some(Time::now()),
            },
        );

        let patch = serde_json::json!({
            "status": {
                "migration": {
                    "phase": PHASE_FAILED,
                    "sourceNodeName": ship.spec.as_ref().and_then(|spec| spec.node_name.clone()),
                    "targetNodeName": self.node_name,
                    "message": message,
                    "timestamp": Time::now(),
                },
                "conditions": conditions
            }
        });

        api.patch_status(name, patch).await?;
        Ok(())
    }
}
