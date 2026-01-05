use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use tugboat_client::WatchEvent;
use tugboat_resources::manifests::core::v1::Ship;

impl ShipReconciler {
    pub(super) async fn reconcile(&self, event: WatchEvent<Ship>) -> Result<(), ReconcileError> {
        match event {
            WatchEvent::Added(ship) => {
                let Some(ship_spec) = &ship.spec else {
                    return Err(ReconcileError::FieldMissing(
                        "v1.Ship".to_string(),
                        "spec".to_string(),
                    ));
                };
                let Some(class) = self.ship_class_api.get(&ship_spec.ship_class).await? else {
                    return Err(ReconcileError::ShipClassNotFound(ship_spec.ship_class));
                };

                self.runtime_operator.run(ship, class).await?;
                Ok(())
            }
            WatchEvent::Modified(_ship) => {
                todo!()
            }
            WatchEvent::Deleted(_ship) => {
                todo!()
            }
        }
    }
}
