use crate::csi::PublishedVolume;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use std::collections::HashMap;
use tracing::{info, warn};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::Ship;

fn finalize_volume_cleanup(
    ship_id: &str,
    cleanup_result: Result<(), ReconcileError>,
    mount_namespace_result: Result<(), ReconcileError>,
) -> Result<(), ReconcileError> {
    if let (Err(_), Err(mount_err)) = (&cleanup_result, &mount_namespace_result) {
        warn!(
            "Failed to clean up mount namespace for ship '{}' after volume cleanup error: {}",
            ship_id, mount_err
        );
    }
    cleanup_result?;
    mount_namespace_result?;
    Ok(())
}

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
        let namespace = meta
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());

        // Tear down CNI network interfaces (best-effort).
        if let Some(spec) = ship.spec.as_ref() {
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

        let mut published_volumes = self.runtime_operator.delete(ship_id.clone()).await?;
        if published_volumes.is_empty() {
            published_volumes = self.csi.load_published_volumes(ship_id).await?;
        }

        let (volumes, controller_publish_secrets) = self
            .volume_cleanup_context_for_ship(&namespace, &ship, &published_volumes)
            .await
            .unwrap_or_else(|err| {
                warn!(
                    "Failed to resolve controller publish secrets for deleting ship '{}': {}",
                    ship_id, err
                );
                (Vec::new(), HashMap::new())
            });

        let cleanup_result = self
            .cleanup_published_volumes_best_effort(&published_volumes, &controller_publish_secrets)
            .await;
        if cleanup_result.is_ok() {
            for volume in &volumes {
                let Some(volume) = volume.persistent_volume_claim() else {
                    continue;
                };
                if let Err(err) = self.mark_volume_attached(&volume.volume_name, false).await {
                    warn!(
                        "Failed to clear attachment status for PersistentVolume '{}': {}",
                        volume.volume_name, err
                    );
                }
            }
        } else {
            warn!(
                "Skipping PersistentVolume attachment status clear for ship '{}' because CSI cleanup failed",
                ship_id
            );
        }
        let mount_namespace_result = self
            .csi
            .cleanup_mount_namespace(ship_id)
            .map_err(ReconcileError::from);
        let cleanup_result = match (cleanup_result, self.cleanup_materialized_volumes(ship_id)) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(err), Ok(())) => Err(err),
            (Ok(()), Err(err)) => Err(err),
            (Err(err), Err(materialized_err)) => {
                warn!(
                    "Failed to clean up materialized volumes for ship '{}': {}",
                    ship_id, materialized_err
                );
                Err(err)
            }
        };
        finalize_volume_cleanup(ship_id, cleanup_result, mount_namespace_result)
    }

    async fn volume_cleanup_context_for_ship(
        &self,
        namespace: &str,
        ship: &Ship,
        published_volumes: &[PublishedVolume],
    ) -> Result<
        (
            Vec<crate::reconciler::volume::VolumeInfo>,
            HashMap<String, HashMap<String, String>>,
        ),
        ReconcileError,
    > {
        let Some(spec) = ship.spec.as_ref() else {
            return Ok((Vec::new(), HashMap::new()));
        };
        let ship_meta = ship.object_meta();
        let ship_name = ship_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            .unwrap_or("");
        let ship_uid = ship_meta
            .as_ref()
            .and_then(|m| m.uid.as_deref())
            .unwrap_or("");
        let volumes = self
            .get_related_volumes(namespace, ship_name, ship_uid, spec)
            .await?;
        let secrets = self
            .controller_publish_secret_map(namespace, &volumes, published_volumes)
            .await?;
        Ok((volumes, secrets))
    }
}

#[cfg(test)]
mod tests {
    use super::finalize_volume_cleanup;
    use crate::csi::CsiError;
    use crate::reconciler::error::ReconcileError;

    #[test]
    fn returns_volume_cleanup_error_after_mount_namespace_attempt() {
        let err = finalize_volume_cleanup(
            "ship-1",
            Err(ReconcileError::PublishedVolumeCleanupFailed(
                "unpublish failed".to_string(),
            )),
            Err(ReconcileError::from(CsiError::MissingVolumeHandle)),
        )
        .unwrap_err();

        match err {
            ReconcileError::PublishedVolumeCleanupFailed(message) => {
                assert_eq!(message, "unpublish failed");
            }
            other => panic!("expected volume cleanup error, got {other}"),
        }
    }

    #[test]
    fn returns_mount_namespace_error_when_volume_cleanup_succeeds() {
        let err = finalize_volume_cleanup(
            "ship-1",
            Ok(()),
            Err(ReconcileError::from(CsiError::MissingVolumeHandle)),
        )
        .unwrap_err();

        match err {
            ReconcileError::Csi(CsiError::MissingVolumeHandle) => {}
            other => panic!("expected mount namespace cleanup error, got {other}"),
        }
    }

    #[test]
    fn succeeds_when_both_cleanup_steps_succeed() {
        assert!(finalize_volume_cleanup("ship-1", Ok(()), Ok(())).is_ok());
    }
}
