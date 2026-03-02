use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use tracing::{info, warn};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::Ship;

impl ShipReconciler {
    pub(crate) async fn reconcile_deleted(&self, ship: Ship) -> Result<(), ReconcileError> {
        let Some(meta) = ship.object_meta() else {
            warn!("Ship has no metadata, cannot delete VM by name");
            return Ok(());
        };
        let Some(name) = &meta.name else {
            warn!("Ship has no name in metadata, cannot delete VM");
            return Ok(());
        };

        info!("Deleting ship: {}", name);
        if let Err(e) = self.runtime_operator.delete(name.clone()).await {
            warn!("Failed to delete ship: {e}");
        }
        Ok(())
    }
}
