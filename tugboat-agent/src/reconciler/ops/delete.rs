use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use std::collections::HashMap;
use tracing::{info, warn};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::Ship;

impl ShipReconciler {
    pub(crate) async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError> {
        let Some(meta) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(ship_id) = &meta.uid else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.uid".to_string(),
            ));
        };

        info!("Deleting ship: {}", ship_id);

        // Tear down CNI network interfaces (best-effort).
        if let Some(spec) = ship.spec.as_ref() {
            let namespace = meta
                .namespace
                .clone()
                .unwrap_or_else(|| "default".to_string());
            match self.get_related_network_classes(&namespace, spec).await {
                Ok(network_classes) => {
                    let networks = self.cni.create_network_configs(ship_id, network_classes);
                    if let Err(err) = self.cni.del(ship_id, networks).await {
                        warn!(
                            "Failed to tear down CNI networks for ship '{}': {}",
                            ship_id, err
                        );
                    }
                }
                Err(err) => {
                    warn!(
                        "Failed to resolve network classes for deleting ship '{}': {}",
                        ship_id, err
                    );
                }
            }
        }

        let (volumes, controller_publish_secrets) = self
            .volume_cleanup_context_for_ship(&ship)
            .await
            .unwrap_or_else(|err| {
                warn!(
                    "Failed to resolve controller publish secrets for deleting ship '{}': {}",
                    ship_id, err
                );
                (Vec::new(), HashMap::new())
            });
        let mut published_volumes = self.runtime_operator.delete(ship_id.clone()).await?;
        if published_volumes.is_empty() {
            published_volumes = self.csi.load_published_volumes(ship_id)?;
        }
        let cleanup_result = self
            .cleanup_published_volumes(&published_volumes, &controller_publish_secrets)
            .await;
        // Always attempt to clear the attachment status regardless of whether CSI
        // unpublish succeeded.  A previous cancelled reconcile may have already
        // completed the CSI teardown, causing cleanup_published_volumes to fail,
        // while the PV status still shows an attached node.
        for volume in &volumes {
            if let Err(err) = self.mark_volume_attached(&volume.volume_name, false).await {
                warn!(
                    "Failed to clear attachment status for PersistentVolume '{}': {}",
                    volume.volume_name, err
                );
            }
        }
        cleanup_result?;
        self.csi.cleanup_mount_namespace(ship_id)?;
        Ok(())
    }

    async fn volume_cleanup_context_for_ship(
        &self,
        ship: &Ship,
    ) -> Result<
        (
            Vec<crate::reconciler::volume::VolumeInfo>,
            HashMap<String, HashMap<String, String>>,
        ),
        ReconcileError,
    > {
        let namespace = ship
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.namespace.clone())
            .unwrap_or_else(|| "default".to_string());
        let Some(spec) = ship.spec.as_ref() else {
            return Ok((Vec::new(), HashMap::new()));
        };
        let volumes = self.get_related_volumes(&namespace, spec).await?;
        let secrets = self.controller_publish_secret_map(&volumes).await?;
        Ok((volumes, secrets))
    }
}
