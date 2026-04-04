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

use crate::cni::NetworkClassInfo;
use crate::csi::READ_WRITE_MANY;
use crate::node_registration::NODE_ARCH_LABEL;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::VolumeInfo;
use crate::runtime::error::RuntimeError;
use async_trait::async_trait;
use std::collections::BTreeSet;
use tracing::{error, info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    Node, NodeCniPluginStatus, Ship, ShipClass, ShipMigrationStatus,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::migrate::VmMigrationPhase;

use super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};

#[async_trait]
pub trait MigrationContext: Send + Sync {
    fn node_name(&self) -> &str;
    async fn has_ship(&self, ship_id: &str) -> bool;
    async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError>;
    async fn preflight_migration(
        &self,
        ship: &Ship,
        target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError>;
    async fn migrate(
        &self,
        ship_id: &str,
        target_address: String,
        target_port: u16,
    ) -> Result<(), RuntimeError>;
    async fn check_migration_status(&self, ship_id: &str)
    -> Result<VmMigrationPhase, RuntimeError>;
    async fn finish_source_migration(&self, ship_id: &str) -> Result<(), RuntimeError>;

    async fn update_migration_status(
        &self,
        namespace: &str,
        name: &str,
        migration: ShipMigrationStatus,
        condition_status: &str,
        condition_message: String,
    ) -> Result<(), ReconcileError>;

    async fn patch_ship(
        &self,
        namespace: &str,
        name: &str,
        patch: serde_json::Value,
    ) -> Result<(), ReconcileError>;
}

#[async_trait]
impl MigrationContext for ShipReconciler {
    fn node_name(&self) -> &str {
        &self.node_name
    }
    async fn has_ship(&self, ship_id: &str) -> bool {
        self.runtime_operator.has_ship(ship_id).await
    }
    async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError> {
        self.reconcile_deleted(ship).await
    }
    async fn preflight_migration(
        &self,
        ship: &Ship,
        target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError> {
        ShipReconciler::preflight_migration(self, ship, target_node_name).await
    }
    async fn migrate(
        &self,
        ship_id: &str,
        target_address: String,
        target_port: u16,
    ) -> Result<(), RuntimeError> {
        self.runtime_operator
            .migrate(ship_id, target_address, target_port)
            .await
    }
    async fn check_migration_status(
        &self,
        ship_id: &str,
    ) -> Result<VmMigrationPhase, RuntimeError> {
        self.runtime_operator.check_migration_status(ship_id).await
    }
    async fn finish_source_migration(&self, ship_id: &str) -> Result<(), RuntimeError> {
        self.runtime_operator.finish_source_migration(ship_id).await
    }

    async fn update_migration_status(
        &self,
        namespace: &str,
        name: &str,
        migration: ShipMigrationStatus,
        condition_status: &str,
        condition_message: String,
    ) -> Result<(), ReconcileError> {
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let patch = serde_json::json!({
            "status": {
                "migration": migration,
                "conditions": [
                    {
                        "status": condition_status,
                        "message": condition_message,
                        "timestamp": Time::now(),
                    }
                ]
            }
        });
        api.patch_status(name, patch).await?;
        Ok(())
    }

    async fn patch_ship(
        &self,
        namespace: &str,
        name: &str,
        patch: serde_json::Value,
    ) -> Result<(), ReconcileError> {
        let api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        api.patch(name, patch).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationPreflight {
    Ready,
    Reject(String),
}

pub struct MigrationStateMachine<'a> {
    context: &'a dyn MigrationContext,
}

impl<'a> MigrationStateMachine<'a> {
    pub fn new(context: &'a dyn MigrationContext) -> Self {
        Self { context }
    }

    async fn mark_source_migration_failed(
        &self,
        namespace: &str,
        name: &str,
        target_node_name: String,
        target_address: Option<String>,
        target_port: Option<u32>,
        message: String,
    ) -> Result<(), ReconcileError> {
        self.context
            .update_migration_status(
                namespace,
                name,
                ShipMigrationStatus {
                    phase: PHASE_FAILED.to_string(),
                    source_node_name: Some(self.context.node_name().to_string()),
                    target_node_name: Some(target_node_name),
                    target_address,
                    target_port,
                    message: message.clone(),
                    timestamp: Some(Time::now()),
                },
                "VmMigrationFailed",
                message,
            )
            .await
    }

    async fn finalize_completed_source_migration(
        &self,
        ship_id: &str,
        namespace: &str,
        name: &str,
        target_node_name: String,
        target_address: String,
        target_port: u32,
    ) -> Result<(), ReconcileError> {
        self.context
            .update_migration_status(
                namespace,
                name,
                ShipMigrationStatus {
                    phase: PHASE_COMPLETED.to_string(),
                    source_node_name: Some(self.context.node_name().to_string()),
                    target_node_name: Some(target_node_name.clone()),
                    target_address: Some(target_address.clone()),
                    target_port: Some(target_port),
                    message:
                        "Live migration completed successfully with preserved guest NIC identity"
                            .to_string(),
                    timestamp: Some(Time::now()),
                },
                "VmMigrated",
                format!(
                    "VM migrated successfully to node '{target_node_name}' with deterministic bridge, interface, and MAC identity"
                ),
            )
            .await?;

        if self.context.has_ship(ship_id).await {
            self.context.finish_source_migration(ship_id).await?;
        }

        let spec_patch = serde_json::json!({
            "spec": {
                "nodeName": target_node_name,
                "targetNodeName": null,
            }
        });
        self.context.patch_ship(namespace, name, spec_patch).await?;
        Ok(())
    }

    pub async fn try_reconcile(&self, ship: &Ship, ship_id: &str) -> Result<bool, ReconcileError> {
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };
        let Some(target_node_name) = ship_spec.target_node_name.clone() else {
            return Ok(false);
        };

        if target_node_name == self.context.node_name() {
            if let Some(status) = &ship.status
                && let Some(migration) = &status.migration
                && migration.phase == PHASE_FAILED
            {
                if self.context.has_ship(ship_id).await {
                    info!(
                        "Migration failed for ship '{}', cleaning up incoming VM on target node",
                        ship_id
                    );
                    if let Err(err) = self.context.reconcile_deleted(ship.clone()).await {
                        error!(
                            "Failed to clean up incoming VM for failed migration '{}': {}",
                            ship_id, err
                        );
                    }
                }
                return Ok(true);
            }
            return Ok(self.context.has_ship(ship_id).await);
        }

        if ship_spec.node_name.as_deref() != Some(self.context.node_name()) {
            return Ok(false);
        }

        let Some(name) = ship.name() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = ship.namespace().unwrap_or("default");
        let migration_status = ship
            .status
            .as_ref()
            .and_then(|status| status.migration.clone());

        let Some(migration_status) = migration_status else {
            match self
                .context
                .preflight_migration(ship, &target_node_name)
                .await?
            {
                MigrationPreflight::Ready => {}
                MigrationPreflight::Reject(reason) => {
                    let message = format!(
                        "Migration preflight failed: {reason}. Source VM remains on the source node."
                    );
                    self.context
                        .update_migration_status(
                            namespace,
                            name,
                            ShipMigrationStatus {
                                phase: PHASE_FAILED.to_string(),
                                source_node_name: ship_spec.node_name.clone(),
                                target_node_name: Some(target_node_name),
                                target_address: None,
                                target_port: None,
                                message: message.clone(),
                                timestamp: Some(Time::now()),
                            },
                            "VmMigrationPreflightFailed",
                            message,
                        )
                        .await?;
                    return Ok(true);
                }
            }
            self.context
                .update_migration_status(
                    namespace,
                    name,
                    ShipMigrationStatus {
                        phase: PHASE_PENDING.to_string(),
                        source_node_name: ship_spec.node_name.clone(),
                        target_node_name: Some(target_node_name),
                        target_address: None,
                        target_port: None,
                        message: "Waiting for target node to prepare migration receiver"
                            .to_string(),
                        timestamp: Some(Time::now()),
                    },
                    "VmMigrationPending",
                    "Waiting for target node to prepare migration receiver".to_string(),
                )
                .await?;
            return Ok(true);
        };

        match migration_status.phase.as_str() {
            PHASE_PENDING => Ok(true),
            PHASE_READY => {
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };

                match self.context.check_migration_status(ship_id).await {
                    Ok(VmMigrationPhase::None) => {
                        if let Err(err) = self
                            .context
                            .migrate(ship_id, target_address.clone(), target_port as u16)
                            .await
                        {
                            let message = format!("Failed to start live migration: {err}");
                            self.mark_source_migration_failed(
                                namespace,
                                name,
                                target_node_name,
                                Some(target_address),
                                Some(target_port),
                                message.clone(),
                            )
                            .await?;
                            return Err(err.into());
                        }

                        self.context
                            .update_migration_status(
                                namespace,
                                name,
                                ShipMigrationStatus {
                                    phase: PHASE_MIGRATING.to_string(),
                                    source_node_name: Some(self.context.node_name().to_string()),
                                    target_node_name: Some(target_node_name),
                                    target_address: Some(target_address),
                                    target_port: Some(target_port),
                                    message: "Live migration in progress".to_string(),
                                    timestamp: Some(Time::now()),
                                },
                                "VmMigrating",
                                "Live migration in progress".to_string(),
                            )
                            .await?;
                        Ok(true)
                    }
                    Ok(VmMigrationPhase::Setup | VmMigrationPhase::Active) => {
                        self.context
                            .update_migration_status(
                                namespace,
                                name,
                                ShipMigrationStatus {
                                    phase: PHASE_MIGRATING.to_string(),
                                    source_node_name: Some(self.context.node_name().to_string()),
                                    target_node_name: Some(target_node_name),
                                    target_address: Some(target_address),
                                    target_port: Some(target_port),
                                    message: "Live migration is already in progress".to_string(),
                                    timestamp: Some(Time::now()),
                                },
                                "VmMigrating",
                                "Live migration is already in progress".to_string(),
                            )
                            .await?;
                        Ok(true)
                    }
                    Ok(VmMigrationPhase::Completed) => {
                        self.finalize_completed_source_migration(
                            ship_id,
                            namespace,
                            name,
                            target_node_name,
                            target_address,
                            target_port,
                        )
                        .await?;
                        Ok(true)
                    }
                    Ok(phase @ (VmMigrationPhase::Failed | VmMigrationPhase::Cancelled)) => {
                        let message = format!(
                            "Live migration did not complete (phase: {phase:?}). Source VM remains authoritative; clean up the target and retry when ready."
                        );
                        self.mark_source_migration_failed(
                            namespace,
                            name,
                            target_node_name,
                            Some(target_address),
                            Some(target_port),
                            message,
                        )
                        .await?;
                        Ok(true)
                    }
                    Err(err) => {
                        warn!(
                            "Failed to check migration status before starting migration for ship '{}': {}",
                            ship_id, err
                        );
                        Ok(true)
                    }
                }
            }
            PHASE_MIGRATING => {
                // Poll migration progress once per reconcile event instead of
                // spinning inside the reconcile loop.
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };
                let target_node_name = migration_status
                    .target_node_name
                    .clone()
                    .unwrap_or(target_node_name);

                let phase = match self.context.check_migration_status(ship_id).await {
                    Ok(phase) => phase,
                    Err(err) => {
                        warn!(
                            "Failed to check migration status for ship '{}': {}",
                            ship_id, err
                        );
                        return Ok(true); // retry on next event
                    }
                };

                match phase {
                    VmMigrationPhase::Completed => {
                        self.finalize_completed_source_migration(
                            ship_id,
                            namespace,
                            name,
                            target_node_name,
                            target_address,
                            target_port,
                        )
                        .await?;
                        Ok(true)
                    }
                    VmMigrationPhase::Failed | VmMigrationPhase::Cancelled => {
                        let message = format!(
                            "Live migration did not complete (phase: {phase:?}). Source VM remains authoritative; clean up the target and retry when ready."
                        );
                        self.mark_source_migration_failed(
                            namespace,
                            name,
                            target_node_name,
                            Some(target_address),
                            Some(target_port),
                            message.clone(),
                        )
                        .await?;
                        Err(ReconcileError::Runtime(RuntimeError::MigrationFailed(
                            message,
                        )))
                    }
                    _ => {
                        // Still active (Setup, Active, None); wait for next event.
                        Ok(true)
                    }
                }
            }
            PHASE_COMPLETED => {
                let Some(target_address) = migration_status.target_address.clone() else {
                    return Ok(true);
                };
                let Some(target_port) = migration_status.target_port else {
                    return Ok(true);
                };
                let target_node_name = migration_status
                    .target_node_name
                    .clone()
                    .unwrap_or(target_node_name);

                if ship_spec.node_name.as_deref() == Some(self.context.node_name()) {
                    self.finalize_completed_source_migration(
                        ship_id,
                        namespace,
                        name,
                        target_node_name,
                        target_address,
                        target_port,
                    )
                    .await?;
                }
                Ok(true)
            }
            // PHASE_FAILED or any unknown terminal phase.
            _ => Ok(true),
        }
    }
}

impl ShipReconciler {
    async fn preflight_migration(
        &self,
        ship: &Ship,
        target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError> {
        let Some(ship_spec) = ship.spec.as_ref() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };

        if ship_spec.node_name.as_deref() == Some(target_node_name) {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' is already hosting the ship"
            )));
        }

        let namespace = ship.namespace().unwrap_or("default");
        let node_api: Api<Node> = Api::all(self.client.clone());
        let Some(target_node) = node_api.get(target_node_name).await? else {
            return Ok(MigrationPreflight::Reject(format!(
                "target node '{target_node_name}' was not found"
            )));
        };

        let Some(ship_class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
            return Err(ReconcileError::ShipClassNotFound(
                ship_spec.ship_class.clone(),
            ));
        };

        if let Some(reason) = validate_target_node_readiness(&target_node) {
            return Ok(MigrationPreflight::Reject(reason));
        }
        if let Some(reason) = validate_target_architecture(&ship_class, &target_node) {
            return Ok(MigrationPreflight::Reject(reason));
        }

        let network_classes = self
            .get_related_network_classes(namespace, ship_spec)
            .await?;
        if let Some(reason) = validate_target_network_capability(&target_node, &network_classes) {
            return Ok(MigrationPreflight::Reject(reason));
        }

        let volumes = self.get_related_volumes(namespace, ship_spec).await?;
        if let Some(reason) = validate_storage_eligibility(&volumes) {
            return Ok(MigrationPreflight::Reject(reason));
        }

        Ok(MigrationPreflight::Ready)
    }
}

fn validate_target_node_readiness(target_node: &Node) -> Option<String> {
    let Some(meta) = target_node.object_meta.as_ref() else {
        return Some("target node is missing metadata".to_string());
    };
    let node_name = meta.name.as_deref().unwrap_or("<unknown>");
    let Some(spec) = target_node.spec.as_ref() else {
        return Some(format!("target node '{node_name}' is missing spec"));
    };

    if !spec.ips.iter().any(|ip| {
        ip.parse::<std::net::IpAddr>()
            .map(|addr| !addr.is_loopback())
            .unwrap_or(false)
    }) {
        return Some(format!(
            "target node '{node_name}' does not advertise a reachable non-loopback IP"
        ));
    }

    let Some(status) = target_node.status.as_ref() else {
        return Some(format!("target node '{node_name}' has no published status"));
    };
    let Some(condition) = status
        .conditions
        .iter()
        .find(|condition| condition.r#type == "CniReady")
    else {
        return Some(format!(
            "target node '{node_name}' does not publish a CniReady condition"
        ));
    };

    if condition.status == "True" {
        None
    } else if condition.message.is_empty() {
        Some(format!("target node '{node_name}' is not CNI-ready"))
    } else {
        Some(format!(
            "target node '{node_name}' is not CNI-ready: {}",
            condition.message
        ))
    }
}

fn validate_target_architecture(ship_class: &ShipClass, target_node: &Node) -> Option<String> {
    let requested = ship_class
        .spec
        .as_ref()
        .and_then(|spec| spec.cpu.as_ref())
        .map(|cpu| normalize_architecture(&cpu.architecture))
        .filter(|arch| !arch.is_empty())?;
    let meta = target_node.object_meta.as_ref()?;
    let node_name = meta.name.as_deref().unwrap_or("<unknown>");
    let Some(actual) = meta
        .labels
        .get(NODE_ARCH_LABEL)
        .map(|arch| normalize_architecture(arch))
    else {
        return Some(format!(
            "target node '{node_name}' does not advertise '{}' label",
            NODE_ARCH_LABEL
        ));
    };

    if requested == actual {
        None
    } else {
        Some(format!(
            "target node '{node_name}' architecture '{actual}' is incompatible with ship class architecture '{requested}'"
        ))
    }
}

fn validate_target_network_capability(
    target_node: &Node,
    network_classes: &[NetworkClassInfo],
) -> Option<String> {
    let statuses = target_node
        .status
        .as_ref()
        .map(|status| status.cni_plugins.as_slice())
        .unwrap_or(&[]);
    let mut required_plugins = BTreeSet::from(["loopback".to_string()]);

    for network_class in network_classes {
        let plugin = normalized_plugin(&network_class.spec);
        match plugin {
            "bridge" => {
                required_plugins.insert("bridge".to_string());
            }
            "flannel" => {
                required_plugins.insert("bridge".to_string());
                required_plugins.insert("flannel".to_string());
                if network_class
                    .spec
                    .flannel
                    .as_ref()
                    .and_then(|flannel| flannel.port_mappings)
                    .unwrap_or(false)
                {
                    required_plugins.insert("portmap".to_string());
                }
            }
            other => {
                return Some(format!(
                    "network class '{}' requires unsupported cniPlugin '{}'",
                    network_class.name, other
                ));
            }
        }
    }

    for plugin in required_plugins {
        if let Some(reason) = require_plugin_ready(statuses, &plugin) {
            return Some(reason);
        }
    }

    None
}

fn validate_storage_eligibility(volumes: &[VolumeInfo]) -> Option<String> {
    for volume in volumes {
        let Some(volume) = volume.persistent_volume_claim() else {
            continue;
        };

        let claim_supports_rwx = volume
            .claim
            .access_modes
            .iter()
            .any(|mode| mode == READ_WRITE_MANY);
        let pv_supports_rwx = volume
            .volume
            .access_modes
            .iter()
            .any(|mode| mode == READ_WRITE_MANY);

        if !claim_supports_rwx || !pv_supports_rwx {
            return Some(format!(
                "persistent volume claim '{}' must use shared storage with '{}' access on both the claim and persistent volume for live migration",
                volume.claim_name, READ_WRITE_MANY
            ));
        }
    }

    None
}

fn normalize_architecture(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => "amd64".to_string(),
        "aarch64" | "arm64" => "arm64".to_string(),
        other => other.to_string(),
    }
}

fn normalized_plugin(spec: &tugboat_resources::manifests::core::v1::NetworkClassSpec) -> &str {
    let plugin = spec.cni_plugin.trim();
    if plugin.is_empty() { "bridge" } else { plugin }
}

fn require_plugin_ready(statuses: &[NodeCniPluginStatus], plugin: &str) -> Option<String> {
    let Some(status) = statuses.iter().find(|status| status.name == plugin) else {
        return Some(format!(
            "target node does not advertise required CNI plugin '{}'",
            plugin
        ));
    };

    if status.ready.unwrap_or(false) {
        None
    } else if status.message.is_empty() {
        Some(format!("required CNI plugin '{}' is not ready", plugin))
    } else {
        Some(format!(
            "required CNI plugin '{}' is not ready: {}",
            plugin, status.message
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tugboat_resources::manifests::core::v1::{
        CsiPersistentVolumeSource, PersistentVolumeClaimSpec, PersistentVolumeSpec, ShipSpec,
        ShipStatus,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[derive(Debug, Clone)]
    struct StatusUpdate {
        migration: ShipMigrationStatus,
        condition_status: String,
        condition_message: String,
    }

    struct FakeContext {
        node_name: String,
        has_ship: bool,
        migration_phase: VmMigrationPhase,
        preflight: MigrationPreflight,
        updates: Mutex<Vec<StatusUpdate>>,
        migrate_calls: Mutex<Vec<(String, u16)>>,
        finish_calls: Mutex<usize>,
        ship_patches: Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait]
    impl MigrationContext for FakeContext {
        fn node_name(&self) -> &str {
            &self.node_name
        }
        async fn has_ship(&self, _ship_id: &str) -> bool {
            self.has_ship
        }
        async fn reconcile_deleted(&self, _ship: Ship) -> Result<(), ReconcileError> {
            Ok(())
        }
        async fn preflight_migration(
            &self,
            _ship: &Ship,
            _target_node_name: &str,
        ) -> Result<MigrationPreflight, ReconcileError> {
            Ok(self.preflight.clone())
        }
        async fn migrate(
            &self,
            _ship_id: &str,
            addr: String,
            port: u16,
        ) -> Result<(), RuntimeError> {
            self.migrate_calls.lock().unwrap().push((addr, port));
            Ok(())
        }
        async fn check_migration_status(
            &self,
            _ship_id: &str,
        ) -> Result<VmMigrationPhase, RuntimeError> {
            Ok(self.migration_phase.clone())
        }
        async fn finish_source_migration(&self, _ship_id: &str) -> Result<(), RuntimeError> {
            *self.finish_calls.lock().unwrap() += 1;
            Ok(())
        }

        async fn update_migration_status(
            &self,
            _namespace: &str,
            _name: &str,
            migration: ShipMigrationStatus,
            condition_status: &str,
            condition_message: String,
        ) -> Result<(), ReconcileError> {
            self.updates.lock().unwrap().push(StatusUpdate {
                migration,
                condition_status: condition_status.to_string(),
                condition_message,
            });
            Ok(())
        }

        async fn patch_ship(
            &self,
            _namespace: &str,
            _name: &str,
            patch: serde_json::Value,
        ) -> Result<(), ReconcileError> {
            self.ship_patches.lock().unwrap().push(patch);
            Ok(())
        }
    }

    fn fake_context(node_name: &str, migration_phase: VmMigrationPhase) -> FakeContext {
        FakeContext {
            node_name: node_name.to_string(),
            has_ship: true,
            migration_phase,
            preflight: MigrationPreflight::Ready,
            updates: Mutex::new(Vec::new()),
            migrate_calls: Mutex::new(Vec::new()),
            finish_calls: Mutex::new(0),
            ship_patches: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn test_migration_not_for_me() {
        let context = fake_context("node-1", VmMigrationPhase::Completed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-2".to_string()),
                target_node_name: Some("node-3".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_migration_source_initial() {
        let context = fake_context("node-1", VmMigrationPhase::Completed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Should initiate migration (update status to Pending) and return true
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_migration_source_ready() {
        let context = fake_context("node-1", VmMigrationPhase::None);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_READY.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(context.migrate_calls.lock().unwrap().len(), 1);
        assert_eq!(
            context
                .updates
                .lock()
                .unwrap()
                .last()
                .map(|update| update.migration.phase.as_str()),
            Some(PHASE_MIGRATING)
        );
    }

    #[tokio::test]
    async fn test_migration_source_migrating_completed() {
        let context = fake_context("node-1", VmMigrationPhase::Completed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.finish_calls.lock().unwrap(), 1);
        assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_migration_target_failed_cleanup() {
        let context = fake_context("target-node", VmMigrationPhase::Failed);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("source-node".to_string()),
                target_node_name: Some("target-node".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_FAILED.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Should handle cleanup on target node and return true
        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_migration_preflight_rejection_marks_ship_failed() {
        let context = FakeContext {
            preflight: MigrationPreflight::Reject("target node is not CNI-ready".to_string()),
            ..fake_context("node-1", VmMigrationPhase::Completed)
        };
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);

        let updates = context.updates.lock().unwrap();
        let update = updates.last().expect("expected migration status update");
        assert_eq!(update.migration.phase, PHASE_FAILED);
        assert_eq!(update.condition_status, "VmMigrationPreflightFailed");
        assert!(
            update
                .condition_message
                .contains("Source VM remains on the source node")
        );
    }

    #[tokio::test]
    async fn test_migration_source_ready_detects_existing_active_migration() {
        let context = fake_context("node-1", VmMigrationPhase::Active);
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_READY.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert!(context.migrate_calls.lock().unwrap().is_empty());
        assert_eq!(
            context
                .updates
                .lock()
                .unwrap()
                .last()
                .map(|update| update.migration.phase.as_str()),
            Some(PHASE_MIGRATING)
        );
    }

    #[tokio::test]
    async fn test_migration_completed_without_runtime_still_finalizes_cutover() {
        let context = FakeContext {
            has_ship: false,
            ..fake_context("node-1", VmMigrationPhase::Completed)
        };
        let sm = MigrationStateMachine::new(&context);
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-1".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-1".to_string()),
                target_node_name: Some("node-2".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_COMPLETED.to_string(),
                    target_address: Some("1.2.3.4".to_string()),
                    target_port: Some(1234),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
        assert!(result);
        assert_eq!(*context.finish_calls.lock().unwrap(), 0);
        assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
    }

    #[test]
    fn normalizes_common_architecture_aliases() {
        assert_eq!(normalize_architecture("x86_64"), "amd64");
        assert_eq!(normalize_architecture("amd64"), "amd64");
        assert_eq!(normalize_architecture("aarch64"), "arm64");
        assert_eq!(normalize_architecture("arm64"), "arm64");
    }

    #[test]
    fn rejects_non_shared_persistent_volumes_for_live_migration() {
        let volume = VolumeInfo::PersistentVolumeClaim(Box::new(
            crate::reconciler::volume::PersistentVolumeClaimVolumeInfo {
                name: "data".to_string(),
                claim_name: "data-pvc".to_string(),
                volume_name: "data-pv".to_string(),
                claim: PersistentVolumeClaimSpec {
                    access_modes: vec!["ReadWriteOnce".to_string()],
                    ..Default::default()
                },
                volume: PersistentVolumeSpec {
                    access_modes: vec!["ReadWriteOnce".to_string()],
                    csi: Some(CsiPersistentVolumeSource::default()),
                    ..Default::default()
                },
                status: None,
                source: CsiPersistentVolumeSource::default(),
            },
        ));

        let reason =
            validate_storage_eligibility(&[volume]).expect("non-shared storage should be rejected");
        assert!(reason.contains(READ_WRITE_MANY));
    }
}
