use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use std::collections::HashMap;
use tracing::info;
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
        let (volumes, controller_publish_secrets) = self
            .volume_cleanup_context_for_ship(&ship)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!(
                    "Failed to resolve controller publish secrets for deleting ship '{}': {}",
                    ship_id,
                    err
                );
                (Vec::new(), HashMap::new())
            });
        let mut published_volumes = self.runtime_operator.delete(ship_id.clone()).await?;
        if published_volumes.is_empty() {
            published_volumes = self.csi.load_published_volumes(ship_id)?;
        }
        self.cleanup_published_volumes(&published_volumes, &controller_publish_secrets)
            .await?;
        for volume in volumes {
            if let Err(err) = self.mark_volume_attached(&volume.volume_name, false).await {
                tracing::warn!(
                    "Failed to clear attachment status for PersistentVolume '{}': {}",
                    volume.volume_name,
                    err
                );
            }
        }
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
