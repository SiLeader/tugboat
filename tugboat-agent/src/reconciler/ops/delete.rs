use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
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
        let mut published_volumes = self.runtime_operator.delete(ship_id.clone()).await?;
        if published_volumes.is_empty() {
            published_volumes = self.csi.load_published_volumes(ship_id)?;
        }
        self.cleanup_published_volumes(&published_volumes).await?;
        self.csi.cleanup_mount_namespace(ship_id)?;
        Ok(())
    }
}
