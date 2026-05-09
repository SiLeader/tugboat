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
use crate::reconciler::volume::VolumeInfo;
use std::collections::HashMap;
use std::future::Future;
use tracing::{error, warn};
use tugboat_client::Api;
use tugboat_resources::ObjectMetaResource;

use super::recovery::{
    CleanupSecretPolicy, best_effort_stale_volume_cleanup, controller_publish_secrets_for_cleanup,
};

impl ShipReconciler {
    pub(super) async fn cleanup_after_volume_setup_error(
        &self,
        ship_id: &str,
        cleanup_targets: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
        context: &str,
    ) -> Vec<String> {
        let mut cleanup_errors = Vec::new();
        if let Err(cleanup_err) = self
            .cleanup_published_volumes(cleanup_targets, controller_publish_secrets)
            .await
        {
            error!("Failed to clean up published volumes after {context}: {cleanup_err}");
            cleanup_errors.push(format!("published volumes: {cleanup_err}"));
        }
        if let Err(cleanup_err) = self.cleanup_materialized_volumes(ship_id) {
            error!("Failed to clean up materialized volumes after {context}: {cleanup_err}");
            cleanup_errors.push(format!("materialized volumes: {cleanup_err}"));
        }
        if let Err(cleanup_err) = self.csi.cleanup_mount_namespace(ship_id) {
            error!("Failed to clean up mount namespace after {context}: {cleanup_err}");
            cleanup_errors.push(format!("mount namespace: {cleanup_err}"));
        }
        cleanup_errors
    }

    pub(super) async fn with_cleanup<Fut, F>(
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
        self.cleanup_published_volumes_with_policy(
            published_volumes,
            controller_publish_secrets,
            CleanupSecretPolicy::RequireControllerPublishSecrets,
        )
        .await
    }

    pub(crate) async fn cleanup_published_volumes_best_effort(
        &self,
        published_volumes: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
    ) -> Result<(), ReconcileError> {
        self.cleanup_published_volumes_with_policy(
            published_volumes,
            controller_publish_secrets,
            CleanupSecretPolicy::BestEffort,
        )
        .await
    }

    async fn cleanup_published_volumes_with_policy(
        &self,
        published_volumes: &[PublishedVolume],
        controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
        policy: CleanupSecretPolicy,
    ) -> Result<(), ReconcileError> {
        let mut errors = Vec::new();
        for volume in published_volumes.iter().rev() {
            let secrets = match controller_publish_secrets_for_cleanup(
                volume,
                controller_publish_secrets,
                policy,
            ) {
                Ok(secrets) => secrets,
                Err(err) => {
                    errors.push(format!("{}: {err}", volume.target_path));
                    continue;
                }
            };
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
        let ship = self.ship_all_api.get(ship_id).await?;
        let namespace = ship
            .as_ref()
            .and_then(|s| s.object_meta().as_ref().and_then(|m| m.namespace.as_ref()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "default".to_string());

        if let Some(spec) = ship.as_ref().and_then(|s| s.spec.as_ref()) {
            match self.get_related_network_classes(&namespace, spec).await {
                Ok(network_classes) => {
                    let networks = self.cni.create_network_configs(ship_id, network_classes);
                    if let Err(err) = self.cni.del(ship_id, networks).await {
                        warn!(
                            "Failed to tear down CNI networks for ship '{}' during add cleanup: {}",
                            ship_id, err
                        );
                    }
                }
                Err(err) => {
                    warn!(
                        "Failed to resolve network classes for ship '{}' during add cleanup: {}",
                        ship_id, err
                    );
                }
            }
        }

        let mut runtime_published_volumes =
            self.runtime_operator.delete(ship_id.to_string()).await?;

        let controller_publish_secrets = self
            .controller_publish_secret_map(
                &namespace,
                volumes,
                if runtime_published_volumes.is_empty() {
                    fallback_published_volumes
                } else {
                    &runtime_published_volumes
                },
            )
            .await?;
        if runtime_published_volumes.is_empty() {
            runtime_published_volumes = self.csi.load_published_volumes(ship_id).await?;
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

    pub(crate) async fn controller_publish_secret_map(
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
