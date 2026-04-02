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

use crate::csi::{
    PublishedAccessType, PublishedVolume, access_type_from_volume_mode, effective_publish_settings,
};
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::ops::PHASE_READY;
use crate::reconciler::ops::add_helpers::{
    validate_recovered_published_volumes, vm_volume_config,
};
use crate::reconciler::reconcile::AppendStatus;
use crate::reconciler::volume::VolumeInfo;
use crate::runtime::RuntimeCreateRequest;
use std::collections::HashMap;
use std::future::Future;
use tracing::{debug, error, info, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    Node, Ship,
    ShipCondition, ShipSpec,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

fn best_effort_stale_volume_cleanup<T>(
    namespace: &str,
    claim_name: &str,
    result: Result<T, ReconcileError>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(err) => {
            // The PVC or its referenced Secret may have already been deleted.
            // We deliberately continue with no entry for this claim so that
            // cleanup_published_volumes falls back to an empty secrets map and
            // still attempts ControllerUnpublishVolume. This is best-effort:
            // CSI drivers that require controller-publish secrets for unpublish
            // may fail, but there is no way to recover the secrets at this point
            // and blocking cleanup would permanently leak the volume attachment.
            warn!(
                "Failed to resolve stale volume '{}' secrets in namespace '{}', \
                 proceeding with empty secrets for best-effort cleanup: {}",
                claim_name, namespace, err
            );
            None
        }
    }
}

fn runtime_fingerprints_for_ship(
    ship_spec: &ShipSpec,
    local_node_name: &str,
) -> Result<super::ShipFingerprints, ReconcileError> {
    if ship_spec.target_node_name.as_deref() == Some(local_node_name) {
        let mut migrated_spec = ship_spec.clone();
        migrated_spec.node_name = Some(local_node_name.to_string());
        migrated_spec.target_node_name = None;
        super::ShipFingerprints::new(&migrated_spec)
    } else {
        super::ShipFingerprints::new(ship_spec)
    }
}

struct VolumeSetupGuard<'a> {
    reconciler: &'a ShipReconciler,
    ship_id: &'a str,
    published_volumes: Vec<PublishedVolume>,
    controller_publish_secrets: HashMap<String, HashMap<String, String>>,
}

impl<'a> VolumeSetupGuard<'a> {
    fn new(reconciler: &'a ShipReconciler, ship_id: &'a str) -> Self {
        Self {
            reconciler,
            ship_id,
            published_volumes: Vec::new(),
            controller_publish_secrets: HashMap::new(),
        }
    }

    async fn cancel(self, context: &str) {
        self.reconciler
            .cleanup_after_volume_setup_error(
                self.ship_id,
                &self.published_volumes,
                &self.controller_publish_secrets,
                context,
            )
            .await;
    }

    fn commit(self) -> Vec<PublishedVolume> {
        self.published_volumes
    }
}

impl ShipReconciler {
    pub(crate) async fn reconcile_added(&self, ship: Ship) -> Result<(), ReconcileError> {
        info!("Starting reconciliation for ship");
        debug!("Checking ship configuration");
        let Some(ship_metadata) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(name) = &ship_metadata.name else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.name".to_string(),
            ));
        };
        let Some(ship_id) = &ship_metadata.uid else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.uid".to_string(),
            ));
        };
        let Some(ship_spec) = &ship.spec else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };
        debug!("Getting ship class named '{}'", ship_spec.ship_class);
        let Some(class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
            return Err(ReconcileError::ShipClassNotFound(
                ship_spec.ship_class.clone(),
            ));
        };
        let namespace = ship_metadata
            .namespace
            .clone()
            .unwrap_or("default".to_string());
        let runtime_fingerprints = runtime_fingerprints_for_ship(ship_spec, &self.node_name)?;

        if self.runtime_operator.is_present(ship_id).await? {
            let volumes = self.get_related_volumes(&namespace, ship_spec).await?;
            let planned_published_volumes = self
                .plan_desired_published_volumes(ship_id, &volumes)
                .await?;
            let published_volumes = validate_recovered_published_volumes(
                ship_id,
                self.csi.load_published_volumes(ship_id)?,
                &planned_published_volumes,
            )?;
            self.runtime_operator
                .register_existing(
                    namespace,
                    name.clone(),
                    ship_id.clone(),
                    runtime_fingerprints,
                    published_volumes,
                )
                .await;
            info!("Recovered existing VM runtime state for ship '{}'", ship_id);
            return Ok(());
        }

        debug!("Getting network classes for ship");
        let network_classes = self
            .get_related_network_classes(&namespace, ship_spec)
            .await?;
        debug!("{} network classes loaded", network_classes.len());
        debug!("Getting volume claims for ship");
        let volumes = self.get_related_volumes(&namespace, ship_spec).await?;
        debug!("{} volumes loaded", volumes.len());
        let stale_published_volumes = self.csi.load_published_volumes(ship_id)?;
        if !stale_published_volumes.is_empty() {
            info!("Cleaning up stale CSI publish state for ship '{}'", ship_id);
            let controller_publish_secrets = self
                .controller_publish_secret_map(&namespace, &volumes, &stale_published_volumes)
                .await?;
            self.cleanup_published_volumes(&stale_published_volumes, &controller_publish_secrets)
                .await?;
            self.csi.cleanup_mount_namespace(ship_id)?;
        }
        self.cleanup_materialized_volumes(ship_id)?;

        {
            debug!("Updating Ship status");
            let api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
            let mut status_ship = ship.clone();
            status_ship.append_status(ShipCondition {
                status: "VmCreating".to_string(),
                message: "Creating new Virtual Machine".to_string(),
                timestamp: Some(Time::now()),
            });
            api.replace_status(name, status_ship).await?;
        }

        debug!("Planning network configurations for ship");
        let networks = self.cni.create_network_configs(ship_id, network_classes);
        let (published_volumes, vm_volumes) =
            self.setup_volumes(ship_id, &namespace, &volumes).await?;
        let incoming_port =
            if ship_spec.target_node_name.as_deref() == Some(self.node_name.as_str()) {
                Some(self.find_available_port().await?)
            } else {
                None
            };
        let ship_for_migration_ready = ship.clone();

        self.with_cleanup(ship_id, published_volumes.as_slice(), &volumes, || {
            let volumes = vm_volumes;
            let published_volumes = published_volumes.clone();
            let ship_for_migration_ready = ship_for_migration_ready.clone();
            async move {
                debug!("Setup virtual machine");
                if let Err(err) = self
                    .runtime_operator
                    .create(RuntimeCreateRequest {
                        ship_id: ship_id.clone(),
                        ship_name: name.clone(),
                        namespace,
                        ship_spec,
                        ship_class: class,
                        incoming_port,
                        networks: networks.iter().map(|n| n.vm.clone()).collect(),
                        volumes,
                        fingerprints: runtime_fingerprints.clone(),
                        published_volumes,
                    })
                    .await
                {
                    return Err(err.into());
                }
                debug!("Creating network resources");
                if let Err(err) = self.cni.add(ship_id, networks).await {
                    return Err(err.into());
                }
                debug!("Starting runtime operator");
                if let Err(err) = self.runtime_operator.start(ship_id).await {
                    return Err(err.into());
                }
                if let Some(port) = incoming_port {
                    self.mark_migration_target_ready(&ship_for_migration_ready, port)
                        .await?;
                }
                Ok(())
            }
        })
        .await
    }

    async fn local_node_address(&self) -> Result<String, ReconcileError> {
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

    async fn find_available_port(&self) -> Result<u16, ReconcileError> {
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

    async fn mark_migration_target_ready(
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

        let patch = serde_json::json!({
            "status": {
                "migration": {
                    "phase": PHASE_READY,
                    "sourceNodeName": ship.spec.as_ref().and_then(|spec| spec.node_name.clone()),
                    "targetNodeName": self.node_name,
                    "targetAddress": target_address,
                    "targetPort": port,
                    "message": "Target VM is ready to accept incoming migration",
                    "timestamp": Time::now(),
                },
                "conditions": [
                    {
                        "status": "VmMigrationTargetReady",
                        "message": format!("VM is listening for incoming migration on {target_address}:{port}"),
                        "timestamp": Time::now(),
                    }
                ]
            }
        });

        api.patch_status(name, patch).await?;
        Ok(())
    }

    async fn setup_volumes(
        &self,
        ship_id: &str,
        namespace: &str,
        volumes: &[VolumeInfo],
    ) -> Result<(Vec<PublishedVolume>, Vec<VmVolumeConfig>), ReconcileError> {
        let mut guard = VolumeSetupGuard::new(self, ship_id);

        let vm_volumes = self.setup_volumes_inner(&mut guard, namespace, volumes).await;

        match vm_volumes {
            Ok(vm_volumes) => Ok((guard.commit(), vm_volumes)),
            Err(err) => {
                guard.cancel("volume setup error").await;
                Err(err)
            }
        }
    }

    async fn setup_volumes_inner(
        &self,
        guard: &mut VolumeSetupGuard<'_>,
        namespace: &str,
        volumes: &[VolumeInfo],
    ) -> Result<Vec<VmVolumeConfig>, ReconcileError> {
        let mut vm_volumes = Vec::new();
        if volumes
            .iter()
            .any(|volume| volume.persistent_volume_claim().is_some())
        {
            self.csi.ensure_mount_namespace(guard.ship_id)?;
        }

        for volume in volumes {
            if let Some(volume) = volume.persistent_volume_claim() {
                let (_, read_only) = effective_publish_settings(
                    &volume.claim.access_modes,
                    &volume.volume.access_modes,
                    volume.source.read_only,
                )?;
                let secrets = self.resolve_csi_secrets(volume).await?;

                let published = self
                    .csi
                    .publish(
                        &self.node_name,
                        guard.ship_id,
                        &volume.name,
                        &volume.claim_name,
                        &volume.volume,
                        &volume.claim,
                        &volume.source,
                        &secrets,
                    )
                    .await?;

                guard
                    .controller_publish_secrets
                    .insert(volume.name.clone(), secrets.controller_publish.clone());
                guard.published_volumes.push(published.clone());

                self.ensure_node_expansion(namespace, volume, &published, &secrets)
                    .await?;
                self.refresh_volume_stats(namespace, volume, &published)
                    .await?;
                self.mark_volume_attached(&volume.volume_name, true).await?;

                vm_volumes.push(vm_volume_config(volume, &published, read_only));
            } else if let Some(volume) = volume.materialized() {
                let path = self.materialize_volume(guard.ship_id, volume)?;
                vm_volumes.push(VmVolumeConfig::filesystem(path, volume.name.clone(), true));
            }
        }
        Ok(vm_volumes)
    }

    async fn cleanup_after_volume_setup_error(
        &self,
        ship_id: &str,
        cleanup_targets: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
        context: &str,
    ) {
        if let Err(cleanup_err) = self
            .cleanup_published_volumes(cleanup_targets, controller_publish_secrets)
            .await
        {
            error!("Failed to clean up published volumes after {context}: {cleanup_err}");
        }
        if let Err(cleanup_err) = self.cleanup_materialized_volumes(ship_id) {
            error!("Failed to clean up materialized volumes after {context}: {cleanup_err}");
        }
        if let Err(cleanup_err) = self.csi.cleanup_mount_namespace(ship_id) {
            error!("Failed to clean up mount namespace after {context}: {cleanup_err}");
        }
    }

    async fn plan_desired_published_volumes(
        &self,
        ship_id: &str,
        volumes: &[VolumeInfo],
    ) -> Result<Vec<PublishedVolume>, ReconcileError> {
        let mut planned = Vec::with_capacity(volumes.len());
        for volume in volumes {
            let Some(volume) = volume.persistent_volume_claim() else {
                continue;
            };
            let access_type = PublishedAccessType::from(access_type_from_volume_mode(
                volume.volume.volume_mode.as_deref(),
            )?);
            let requires_staging = self
                .csi
                .driver_requires_staging(&volume.source.driver)
                .await?;
            planned.push(self.csi.plan_published_volume(
                ship_id,
                &volume.name,
                &volume.claim_name,
                &volume.source,
                access_type,
                requires_staging,
            )?);
        }
        Ok(planned)
    }

    async fn with_cleanup<Fut, F>(
        &self,
        ship_id: &str,
        published_volumes: &[PublishedVolume],
        volumes: &[VolumeInfo],
        f: F,
    ) -> Result<(), ReconcileError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), ReconcileError>>,
    {
        match f().await {
            Ok(_) => Ok(()),
            Err(e) => {
                if let Err(cleanup_err) = self
                    .cleanup_runtime_and_published_volumes(ship_id, published_volumes, volumes)
                    .await
                {
                    error!(
                        "Failed to clean up runtime and published volumes after start error: {cleanup_err}"
                    );
                }
                Err(e)
            }
        }
    }

    pub(crate) async fn cleanup_published_volumes(
        &self,
        published_volumes: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
    ) -> Result<(), ReconcileError> {
        let mut errors = Vec::new();
        for volume in published_volumes.iter().rev() {
            let secrets = controller_publish_secrets
                .get(&volume.claim_name)
                .cloned()
                .unwrap_or_default();
            if let Err(err) = self.csi.unpublish(volume, &self.node_name, &secrets).await {
                error!(
                    "Failed to unpublish CSI volume '{}' for ship mount namespace '{}': {err}",
                    volume.target_path, volume.mount_namespace_path
                );
                errors.push(format!("{}: {err}", volume.target_path));
            }
        }
        if !errors.is_empty() {
            return Err(ReconcileError::PublishedVolumeCleanupFailed(
                errors.join("; "),
            ));
        }
        Ok(())
    }

    async fn cleanup_runtime_and_published_volumes(
        &self,
        ship_id: &str,
        fallback_published_volumes: &[PublishedVolume],
        volumes: &[VolumeInfo],
    ) -> Result<(), ReconcileError> {
        let mut runtime_published_volumes =
            self.runtime_operator.delete(ship_id.to_string()).await?;

        let ship = self.ship_all_api.get(ship_id).await?;
        let namespace = ship
            .as_ref()
            .and_then(|s| s.object_meta().as_ref().and_then(|m| m.namespace.as_ref()))
            .map(|s| s.as_str())
            .unwrap_or("default");

        let controller_publish_secrets = self
            .controller_publish_secret_map(
                namespace,
                volumes,
                if runtime_published_volumes.is_empty() {
                    fallback_published_volumes
                } else {
                    &runtime_published_volumes
                },
            )
            .await?;
        if runtime_published_volumes.is_empty() {
            runtime_published_volumes = self.csi.load_published_volumes(ship_id)?;
        }
        if runtime_published_volumes.is_empty() {
            self.cleanup_published_volumes(fallback_published_volumes, &controller_publish_secrets)
                .await?;
        } else {
            self.cleanup_published_volumes(&runtime_published_volumes, &controller_publish_secrets)
                .await?;
        }
        self.cleanup_materialized_volumes(ship_id)?;
        self.csi.cleanup_mount_namespace(ship_id)?;
        Ok(())
    }

    pub(super) async fn controller_publish_secret_map(
        &self,
        namespace: &str,
        volumes: &[VolumeInfo],
        stale_volumes: &[PublishedVolume],
    ) -> Result<HashMap<String, HashMap<String, String>>, ReconcileError> {
        let mut secrets = HashMap::with_capacity(volumes.len() + stale_volumes.len());
        for volume in volumes {
            let Some(volume) = volume.persistent_volume_claim() else {
                continue;
            };
            let resolved = self.resolve_csi_secrets(volume).await?;
            secrets.insert(volume.name.clone(), resolved.controller_publish);
        }

        for published in stale_volumes {
            if secrets.contains_key(&published.claim_name) {
                continue;
            }

            let pvc_name = published.effective_pvc_name();
            let Some(volume_info) = best_effort_stale_volume_cleanup(
                namespace,
                &published.claim_name,
                self.load_persistent_volume_claim(
                    namespace,
                    &Api::namespaced(self.client.clone(), namespace),
                    &Api::all(self.client.clone()),
                    published.claim_name.clone(),
                    pvc_name.to_string(),
                )
                .await,
            ) else {
                continue;
            };
            let Some(resolved) = best_effort_stale_volume_cleanup(
                namespace,
                &published.claim_name,
                self.resolve_csi_secrets(&volume_info).await,
            ) else {
                continue;
            };
            secrets.insert(published.claim_name.clone(), resolved.controller_publish);
        }

        Ok(secrets)
    }
}

#[cfg(test)]
mod tests {
    use super::best_effort_stale_volume_cleanup;
    use crate::reconciler::error::ReconcileError;

    #[test]
    fn best_effort_stale_volume_cleanup_keeps_success_values() {
        let result = best_effort_stale_volume_cleanup("default", "claim-1", Ok(42_u8));

        assert_eq!(result, Some(42));
    }

    #[test]
    fn best_effort_stale_volume_cleanup_drops_errors() {
        let result: Option<()> = best_effort_stale_volume_cleanup(
            "default",
            "claim-1",
            Err(ReconcileError::FieldMissing(
                "v1.PersistentVolumeClaim".to_string(),
                "metadata.name".to_string(),
            )),
        );

        assert_eq!(result, None);
    }
}
