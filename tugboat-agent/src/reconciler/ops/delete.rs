use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
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
        let namespace = meta.namespace.clone().unwrap_or("default".to_string());

        info!("Deleting ship: {}", ship_id);
        let (mut published_volumes, runtime_deleted) = match self
            .runtime_operator
            .delete(ship_id.clone())
            .await
        {
            Ok(published_volumes) => (published_volumes, true),
            Err(err) => {
                warn!(
                    "Failed to stop runtime for deleted ship '{ship_id}': {err}. Continuing CSI cleanup."
                );
                (
                    self.runtime_operator.take_published_volumes(ship_id).await,
                    false,
                )
            }
        };
        if published_volumes.is_empty() {
            let Some(ship_spec) = &ship.spec else {
                return Err(ReconcileError::FieldMissing(
                    "v1.Ship".to_string(),
                    "spec".to_string(),
                ));
            };
            let resolved_volumes = self.get_related_volumes(&namespace, ship_spec).await?;
            published_volumes = resolved_volumes
                .into_iter()
                .map(|volume| {
                    self.csi
                        .plan_published_volume(ship_id, &volume.claim_name, &volume.source)
                })
                .collect::<Result<Vec<_>, _>>()?;
        }
        self.cleanup_published_volumes(&published_volumes).await?;
        if runtime_deleted {
            self.csi.cleanup_mount_namespace(ship_id)?;
        }
        Ok(())
    }
}
