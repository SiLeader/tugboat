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
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::reconcile::AppendStatus;
use crate::runtime::RuntimeCreateRequest;
use tracing::{debug, error, info};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

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

        debug!("Getting network classes for ship");
        let network_classes = self
            .get_related_network_classes(&namespace, ship_spec)
            .await?;
        debug!("{} network classes loaded", network_classes.len());
        debug!("Getting volume claims for ship");
        let volumes = self.get_related_volumes(&namespace, ship_spec).await?;
        debug!("{} volumes loaded", volumes.len());

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
        let mut published_volumes = Vec::new();
        let mut vm_volumes = Vec::new();
        if !volumes.is_empty() {
            self.csi.ensure_mount_namespace(ship_id)?;
        }
        for volume in &volumes {
            match self
                .csi
                .publish(
                    ship_id,
                    &volume.claim_name,
                    &volume.volume,
                    &volume.claim,
                    &volume.source,
                )
                .await
            {
                Ok(published) => {
                    vm_volumes.push(VmVolumeConfig {
                        host_path: published.target_path.clone(),
                        format: "raw".to_string(),
                        read_only: volume.source.read_only,
                    });
                    published_volumes.push(published);
                }
                Err(err) => {
                    if let Err(cleanup_err) =
                        self.cleanup_published_volumes(&published_volumes).await
                    {
                        error!(
                            "Failed to roll back published volumes after publish error: {cleanup_err}"
                        );
                    }
                    if let Err(cleanup_err) = self.csi.cleanup_mount_namespace(ship_id) {
                        error!(
                            "Failed to clean up mount namespace after publish error: {cleanup_err}"
                        );
                    }
                    return Err(err.into());
                }
            }
        }

        debug!("Setup virtual machine");
        if let Err(err) = self
            .runtime_operator
            .create(RuntimeCreateRequest {
                ship_id: ship_id.clone(),
                ship_name: name.clone(),
                namespace,
                ship_spec,
                ship_class: class,
                networks: networks.iter().map(|n| n.vm.clone()).collect(),
                volumes: vm_volumes,
                published_volumes: published_volumes.clone(),
            })
            .await
        {
            if let Err(cleanup_err) = self.cleanup_published_volumes(&published_volumes).await {
                error!("Failed to clean up published volumes after create error: {cleanup_err}");
            }
            return Err(err.into());
        }
        debug!("Creating network resources");
        if let Err(err) = self.cni.add(ship_id, networks).await {
            if let Err(cleanup_err) = self
                .cleanup_runtime_and_published_volumes(ship_id, &published_volumes)
                .await
            {
                error!(
                    "Failed to clean up runtime and published volumes after network error: {cleanup_err}"
                );
            }
            return Err(err.into());
        }
        debug!("Starting runtime operator");
        if let Err(err) = self.runtime_operator.start(ship_id).await {
            if let Err(cleanup_err) = self
                .cleanup_runtime_and_published_volumes(ship_id, &published_volumes)
                .await
            {
                error!(
                    "Failed to clean up runtime and published volumes after start error: {cleanup_err}"
                );
            }
            return Err(err.into());
        }
        Ok(())
    }

    pub(crate) async fn cleanup_published_volumes(
        &self,
        published_volumes: &[PublishedVolume],
    ) -> Result<(), ReconcileError> {
        for volume in published_volumes.iter().rev() {
            self.csi.unpublish(volume).await?;
        }
        Ok(())
    }

    async fn cleanup_runtime_and_published_volumes(
        &self,
        ship_id: &str,
        fallback_published_volumes: &[PublishedVolume],
    ) -> Result<(), ReconcileError> {
        let (runtime_published_volumes, runtime_deleted) = match self
            .runtime_operator
            .delete(ship_id.to_string())
            .await
        {
            Ok(published_volumes) => (published_volumes, true),
            Err(err) => {
                error!(
                    "Failed to stop runtime while rolling back ship '{ship_id}': {err}. Continuing CSI cleanup."
                );
                (
                    self.runtime_operator.take_published_volumes(ship_id).await,
                    false,
                )
            }
        };
        if runtime_published_volumes.is_empty() {
            self.cleanup_published_volumes(fallback_published_volumes)
                .await?;
        } else {
            self.cleanup_published_volumes(&runtime_published_volumes)
                .await?;
        }
        if runtime_deleted {
            self.csi.cleanup_mount_namespace(ship_id)?;
        }
        Ok(())
    }
}
