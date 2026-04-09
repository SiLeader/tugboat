use crate::base::TugboatController;
use crate::config::ControllerManagerConfig;
use crate::error::ControllerError;
use crate::provisioning::{
    build_persistent_volume, claim_access_modes, claim_access_type, dynamic_volume_name,
    existing_pv_matches_claim, load_secret_reference, persistent_volume_capacity_bytes,
    provisioner_config, pvc_identity, reclaim_policy_from_storage_class, requested_capacity_bytes,
    storage_class_csi_config, storage_class_provisioner,
};
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_csi_operator::TugboatCsiOperator;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimStatus, StorageClass,
};

#[derive(Clone)]
struct PvcProvisionerReconciler {
    client: TugboatClient,
    csi_operator: TugboatCsiOperator,
    config: ControllerManagerConfig,
}

pub(crate) struct PvcProvisionerController {
    controller: Controller<PersistentVolumeClaim>,
    reconciler: PvcProvisionerReconciler,
}

impl PvcProvisionerController {
    pub(crate) fn new(
        client: TugboatClient,
        csi_operator: TugboatCsiOperator,
        config: ControllerManagerConfig,
    ) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: PvcProvisionerReconciler {
                client,
                csi_operator,
                config,
            },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for PvcProvisionerController {
    fn name(&self) -> &str {
        "pvc-provisioner"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<PersistentVolumeClaim> for PvcProvisionerReconciler {
    type Error = ControllerError;

    async fn reconcile(
        &self,
        event: ReconcileEvent<PersistentVolumeClaim>,
    ) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(pvc) => self.reconcile_applied(pvc).await,
            ReconcileEvent::Deleted(pvc) => self.reconcile_deleted(pvc).await,
        }
    }
}

impl PvcProvisionerReconciler {
    fn requeue_action(&self) -> Action {
        Action::requeue(self.config.csi.requeue_interval())
    }

    async fn reconcile_applied(
        &self,
        pvc: PersistentVolumeClaim,
    ) -> Result<Action, ControllerError> {
        if pvc.deletion_timestamp().is_some() {
            return Ok(Action::await_change());
        }

        let (namespace, name, uid) = pvc_identity(&pvc)?;
        let spec =
            pvc.spec
                .as_ref()
                .ok_or_else(|| ControllerError::MissingPersistentVolumeClaimSpec {
                    namespace: namespace.clone(),
                    name: name.clone(),
                })?;

        let Some(storage_class_name) = spec
            .storage_class_name
            .clone()
            .filter(|value| !value.is_empty())
        else {
            return Ok(Action::await_change());
        };

        let storage_class_api: Api<StorageClass> = Api::all(self.client.clone());
        let Some(storage_class) = storage_class_api.get(&storage_class_name).await? else {
            tracing::warn!(
                "StorageClass '{}' referenced by PersistentVolumeClaim '{}/{}' is not available yet",
                storage_class_name,
                namespace,
                name
            );
            return Ok(self.requeue_action());
        };

        let (provisioner, parameters) = storage_class_provisioner(&storage_class)?;
        let csi_config = storage_class_csi_config(&storage_class)?;
        let Some(provisioner_config) = provisioner_config(&self.config, &provisioner) else {
            return Ok(Action::await_change());
        };
        let reclaim_policy = reclaim_policy_from_storage_class(&storage_class)?;
        let access_modes = claim_access_modes(&namespace, &name, &spec.access_modes)?;
        let access_type = claim_access_type(&namespace, &name, spec.volume_mode.as_deref())?;
        let requested_capacity_bytes = requested_capacity_bytes(&namespace, &name, spec)?;
        let controller_create_secrets = load_secret_reference(
            &self.client,
            csi_config.controller_create_secret_ref.as_ref(),
        )
        .await?;
        let bound_volume_name = spec.volume_name.clone().filter(|value| !value.is_empty());
        let pv_name = bound_volume_name
            .clone()
            .unwrap_or_else(|| dynamic_volume_name(&namespace, &name, uid.as_deref()));

        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let mut created_pv = false;
        let mut provisioned_volume_id: Option<String> = None;
        if let Some(existing) = pv_api.get(&pv_name).await? {
            if !existing_pv_matches_claim(&existing, &namespace, &name, &storage_class_name)? {
                return Err(ControllerError::ExistingVolumeConflict {
                    name: pv_name.clone(),
                    namespace: namespace.clone(),
                    claim: name.clone(),
                });
            }
        } else if bound_volume_name.is_some() {
            tracing::warn!(
                "PersistentVolumeClaim '{}/{}' references missing PersistentVolume '{}'",
                namespace,
                name,
                pv_name
            );
            return Ok(self.requeue_action());
        } else {
            let provisioned_volume = self
                .csi_operator
                .create_volume(
                    &provisioner_config.socket_path,
                    pv_name.clone(),
                    requested_capacity_bytes,
                    parameters,
                    access_modes.clone(),
                    access_type,
                    controller_create_secrets.clone(),
                    csi_config.mount_options.clone(),
                )
                .await?;
            let volume_id = provisioned_volume.volume_id.clone();
            provisioned_volume_id = Some(volume_id.clone());

            let persistent_volume = build_persistent_volume(
                &pv_name,
                &namespace,
                &name,
                storage_class_name.clone(),
                provisioner,
                reclaim_policy,
                spec.access_modes.clone(),
                spec.volume_mode.clone(),
                Some(provisioned_volume.capacity_bytes),
                csi_config.clone(),
                volume_id.clone(),
                provisioned_volume.volume_context.clone(),
            );

            match pv_api.create(persistent_volume).await {
                Ok(_) => {
                    created_pv = true;
                }
                Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                    let Some(existing) = pv_api.get(&pv_name).await? else {
                        self.cleanup_csi_volume_with_retry(
                            &provisioner_config.socket_path,
                            &volume_id,
                            &controller_create_secrets,
                        )
                        .await;
                        return Err(ControllerError::ExistingVolumeConflict {
                            name: pv_name.clone(),
                            namespace: namespace.clone(),
                            claim: name.clone(),
                        });
                    };
                    if !existing_pv_matches_claim(
                        &existing,
                        &namespace,
                        &name,
                        &storage_class_name,
                    )? {
                        self.cleanup_csi_volume_with_retry(
                            &provisioner_config.socket_path,
                            &volume_id,
                            &controller_create_secrets,
                        )
                        .await;
                        return Err(ControllerError::ExistingVolumeConflict {
                            name: pv_name.clone(),
                            namespace: namespace.clone(),
                            claim: name.clone(),
                        });
                    }
                }
                Err(err) => {
                    self.cleanup_csi_volume_with_retry(
                        &provisioner_config.socket_path,
                        &volume_id,
                        &controller_create_secrets,
                    )
                    .await;
                    return Err(err.into());
                }
            }
        }

        let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), &namespace);
        let Some(mut latest) = pvc_api.get(&name).await? else {
            if created_pv {
                tracing::warn!(
                    "PersistentVolumeClaim '{}/{}' no longer exists; cleaning up orphaned PersistentVolume '{}'",
                    namespace,
                    name,
                    pv_name
                );
                self.cleanup_orphaned_volume(
                    &provisioner_config.socket_path,
                    provisioned_volume_id.as_deref(),
                    &controller_create_secrets,
                    &pv_api,
                    &pv_name,
                )
                .await;
            }
            return Ok(Action::await_change());
        };
        if latest.deletion_timestamp().is_some() {
            if created_pv {
                tracing::warn!(
                    "PersistentVolumeClaim '{}/{}' is being deleted; cleaning up orphaned PersistentVolume '{}'",
                    namespace,
                    name,
                    pv_name
                );
                self.cleanup_orphaned_volume(
                    &provisioner_config.socket_path,
                    provisioned_volume_id.as_deref(),
                    &controller_create_secrets,
                    &pv_api,
                    &pv_name,
                )
                .await;
            }
            return Ok(Action::await_change());
        }

        let Some(mut current_pv) = pv_api.get(&pv_name).await? else {
            tracing::warn!(
                "PersistentVolume '{}' for PersistentVolumeClaim '{}/{}' is not available yet",
                pv_name,
                namespace,
                name
            );
            return Ok(self.requeue_action());
        };

        let latest_spec = latest.spec.as_mut().ok_or_else(|| {
            ControllerError::MissingPersistentVolumeClaimSpec {
                namespace: namespace.clone(),
                name: name.clone(),
            }
        })?;
        let mut latest_changed = false;
        if let Some(existing_volume_name) = latest_spec
            .volume_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            if existing_volume_name != pv_name.as_str() {
                if created_pv {
                    tracing::warn!(
                        "PersistentVolumeClaim '{}/{}' is already bound to '{}'; cleaning up orphaned PersistentVolume '{}'",
                        namespace,
                        name,
                        existing_volume_name,
                        pv_name
                    );
                    self.cleanup_orphaned_volume(
                        &provisioner_config.socket_path,
                        provisioned_volume_id.as_deref(),
                        &controller_create_secrets,
                        &pv_api,
                        &pv_name,
                    )
                    .await;
                }
                return Ok(Action::await_change());
            }
        } else {
            latest_spec.volume_name = Some(pv_name.clone());
            latest_changed = true;
        }

        let mut effective_capacity_bytes = persistent_volume_capacity_bytes(&current_pv)?;
        let mut resize_pending = current_pv
            .status
            .as_ref()
            .and_then(|status| status.node_expansion_required)
            .unwrap_or(false);
        if let Some(requested_capacity_bytes) = requested_capacity_bytes {
            let needs_resize = match effective_capacity_bytes {
                Some(current_capacity) => requested_capacity_bytes > current_capacity,
                None => true,
            };
            if needs_resize {
                if !csi_config.allow_volume_expansion {
                    latest_changed |= apply_pvc_status(
                        &mut latest,
                        "ResizeRejected",
                        effective_capacity_bytes,
                        false,
                    );
                    if latest_changed {
                        pvc_api.replace(&name, latest).await?;
                    }
                    return Ok(Action::await_change());
                }

                let (volume_id, fs_type, controller_expand_secret_ref) = {
                    let spec = current_pv.spec.as_ref().ok_or_else(|| {
                        ControllerError::MissingPersistentVolumeSpec {
                            name: pv_name.clone(),
                        }
                    })?;
                    let Some(csi) = spec.csi.as_ref() else {
                        tracing::warn!(
                            "PersistentVolume '{}' bound to PersistentVolumeClaim '{}/{}' is not CSI-backed; controller resize is not available",
                            pv_name,
                            namespace,
                            name
                        );
                        latest_changed |= apply_pvc_status(
                            &mut latest,
                            "ResizeRejected",
                            effective_capacity_bytes,
                            false,
                        );
                        if latest_changed {
                            pvc_api.replace(&name, latest).await?;
                        }
                        return Ok(Action::await_change());
                    };
                    if csi.volume_handle.is_empty() {
                        return Err(ControllerError::MissingVolumeHandle {
                            name: pv_name.clone(),
                        });
                    }
                    (
                        csi.volume_handle.clone(),
                        csi.fs_type.clone(),
                        csi.controller_expand_secret_ref.as_ref(),
                    )
                };
                let controller_expand_secrets =
                    load_secret_reference(&self.client, controller_expand_secret_ref).await?;
                let mount_flags =
                    if matches!(access_type, tugboat_csi_operator::CsiAccessType::Filesystem) {
                        csi_config.mount_options.clone()
                    } else {
                        Vec::new()
                    };
                let expanded = self
                    .csi_operator
                    .controller_expand(
                        &provisioner_config.socket_path,
                        volume_id,
                        requested_capacity_bytes,
                        access_modes[0],
                        access_type,
                        fs_type,
                        mount_flags,
                        controller_expand_secrets,
                    )
                    .await?;

                if let Some(spec) = current_pv.spec.as_mut() {
                    spec.capacity_bytes = Some(expanded.capacity_bytes);
                }
                apply_pv_status(
                    &mut current_pv,
                    if expanded.node_expansion_required {
                        "NodeExpansionPending"
                    } else {
                        "Bound"
                    },
                    Some(expanded.capacity_bytes),
                    expanded.node_expansion_required,
                );
                pv_api.replace(&pv_name, current_pv).await?;
                effective_capacity_bytes = Some(expanded.capacity_bytes);
                resize_pending = expanded.node_expansion_required;
            }
        }

        latest_changed |= apply_pvc_status(
            &mut latest,
            if resize_pending {
                "NodeExpansionPending"
            } else {
                "Bound"
            },
            effective_capacity_bytes,
            resize_pending,
        );
        if latest_changed {
            pvc_api.replace(&name, latest).await?;
        }
        Ok(Action::await_change())
    }

    async fn reconcile_deleted(
        &self,
        _pvc: PersistentVolumeClaim,
    ) -> Result<Action, ControllerError> {
        // PV deletion is handled by PV Cleanup Controller, which detects
        // that the bound claim no longer exists and cleans up the PV and
        // its backing CSI volume via its finalizer.
        Ok(Action::await_change())
    }

    async fn cleanup_orphaned_volume(
        &self,
        socket_path: &str,
        volume_id: Option<&str>,
        volume_delete_secrets: &std::collections::HashMap<String, String>,
        pv_api: &Api<PersistentVolume>,
        pv_name: &str,
    ) {
        if let Some(volume_id) = volume_id
            && let Err(cleanup_err) = self
                .csi_operator
                .delete_volume(
                    socket_path,
                    volume_id.to_string(),
                    volume_delete_secrets.clone(),
                )
                .await
        {
            tracing::warn!(
                "Failed to clean up orphaned CSI volume '{}': {}",
                volume_id,
                cleanup_err
            );
        }
        if let Err(cleanup_err) = pv_api.delete(pv_name).await {
            tracing::warn!(
                "Failed to clean up orphaned PersistentVolume '{}': {}",
                pv_name,
                cleanup_err
            );
        }
    }

    /// Attempts to delete a CSI volume with up to 3 retries on failure.
    async fn cleanup_csi_volume_with_retry(
        &self,
        socket_path: &str,
        volume_id: &str,
        secrets: &std::collections::HashMap<String, String>,
    ) {
        const MAX_RETRIES: usize = 3;
        for attempt in 1..=MAX_RETRIES {
            match self
                .csi_operator
                .delete_volume(socket_path, volume_id.to_string(), secrets.clone())
                .await
            {
                Ok(()) => return,
                Err(err) if attempt < MAX_RETRIES => {
                    tracing::warn!(
                        "Failed to clean up orphaned CSI volume '{}' (attempt {}/{}): {}",
                        volume_id,
                        attempt,
                        MAX_RETRIES,
                        err
                    );
                }
                Err(err) => {
                    tracing::error!(
                        "Failed to clean up orphaned CSI volume '{}' after {} attempts: {}; \
                         manual intervention may be required",
                        volume_id,
                        MAX_RETRIES,
                        err
                    );
                }
            }
        }
    }
}

fn apply_pvc_status(
    pvc: &mut PersistentVolumeClaim,
    phase: &str,
    capacity_bytes: Option<i64>,
    resize_pending: bool,
) -> bool {
    let status = pvc
        .status
        .get_or_insert_with(PersistentVolumeClaimStatus::default);
    let normalized_capacity_bytes = capacity_bytes.filter(|value| *value > 0);
    let mut changed = false;
    if status.phase.as_deref() != Some(phase) {
        status.phase = Some(phase.to_string());
        changed = true;
    }
    if status.capacity_bytes != normalized_capacity_bytes {
        status.capacity_bytes = normalized_capacity_bytes;
        changed = true;
    }
    if status.resize_pending != Some(resize_pending) {
        status.resize_pending = Some(resize_pending);
        changed = true;
    }
    changed
}

fn apply_pv_status(
    pv: &mut PersistentVolume,
    phase: &str,
    capacity_bytes: Option<i64>,
    node_expansion_required: bool,
) {
    let status = pv.status.get_or_insert_with(Default::default);
    status.phase = Some(phase.to_string());
    status.capacity_bytes = capacity_bytes.filter(|value| *value > 0);
    status.node_expansion_required = Some(node_expansion_required);
}
