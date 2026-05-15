use crate::base::TugboatController;
use crate::config::ControllerManagerConfig;
use crate::error::ControllerError;
use crate::provisioning::{
    build_persistent_volume, claim_access_modes, claim_access_type, dynamic_volume_name,
    existing_pv_matches_claim, is_retryable_csi_cleanup_error, load_secret_reference,
    persistent_volume_capacity_bytes, provisioner_config, pvc_identity,
    reclaim_policy_from_storage_class, requested_capacity_bytes, storage_class_csi_config,
    storage_class_provisioner,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_csi_operator::{ControllerCapability, CsiVolumeContentSource, TugboatCsiOperator};
use tugboat_resources::manifests::core::v1::{
    Node, PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimCondition,
    PersistentVolumeClaimStatus, StorageClass,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_resources::manifests::snapshot::v1::{VolumeSnapshot, VolumeSnapshotContent};
use tugboat_resources::{ObjectMetaResource, SELECTED_NODE_ANNOTATION};

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

#[derive(Default)]
struct WffcAccessibilityTopologies {
    waiting: bool,
    requeue: bool,
    topologies: Vec<HashMap<String, String>>,
}

enum DataSourceResolution {
    Ready(Option<CsiVolumeContentSource>),
    Pending(Action),
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

    async fn wffc_accessibility_topologies(
        &self,
        pvc: &PersistentVolumeClaim,
        storage_class: &StorageClass,
        namespace: &str,
        name: &str,
    ) -> Result<WffcAccessibilityTopologies, ControllerError> {
        let Some(storage_class_spec) = storage_class.spec.as_ref() else {
            return Ok(WffcAccessibilityTopologies::default());
        };
        if storage_class_spec.volume_binding_mode.as_deref() != Some("WaitForFirstConsumer") {
            return Ok(WffcAccessibilityTopologies::default());
        }
        if pvc
            .spec
            .as_ref()
            .and_then(|spec| spec.volume_name.as_deref())
            .is_some_and(|value| !value.is_empty())
        {
            return Ok(WffcAccessibilityTopologies::default());
        }

        let Some(selected_node) = pvc
            .object_meta
            .as_ref()
            .and_then(|meta| meta.annotations.get(SELECTED_NODE_ANNOTATION))
            .map(String::as_str)
            .filter(|value| !value.is_empty())
        else {
            return Ok(WffcAccessibilityTopologies {
                waiting: true,
                requeue: false,
                topologies: Vec::new(),
            });
        };

        let node_api: Api<Node> = Api::all(self.client.clone());
        let Some(node) = node_api.get(selected_node).await? else {
            tracing::warn!(
                "PersistentVolumeClaim '{}/{}' selected node '{}' is not available yet",
                namespace,
                name,
                selected_node
            );
            return Ok(WffcAccessibilityTopologies {
                waiting: true,
                requeue: true,
                topologies: Vec::new(),
            });
        };
        let Some(labels) = node.object_meta.as_ref().map(|meta| &meta.labels) else {
            return Ok(WffcAccessibilityTopologies::default());
        };
        let topology = selected_topology_labels(labels, storage_class);
        Ok(WffcAccessibilityTopologies {
            waiting: false,
            requeue: false,
            topologies: if topology.is_empty() {
                Vec::new()
            } else {
                vec![topology]
            },
        })
    }

    async fn apply_waiting_for_first_consumer(
        &self,
        pvc: &PersistentVolumeClaim,
        namespace: &str,
        name: &str,
    ) -> Result<(), ControllerError> {
        let mut updated = pvc.clone();
        let changed = apply_pvc_status(&mut updated, "Pending", None, false)
            | upsert_pvc_condition(
                &mut updated,
                "WaitingForFirstConsumer",
                "Waiting for the scheduler to select a node before provisioning.",
            );
        if changed {
            let pvc_api: Api<PersistentVolumeClaim> =
                Api::namespaced(self.client.clone(), namespace);
            pvc_api.replace_status(name, updated).await?;
        }
        Ok(())
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
        let accessibility_topologies = self
            .wffc_accessibility_topologies(&pvc, &storage_class, &namespace, &name)
            .await?;
        if accessibility_topologies.waiting {
            self.apply_waiting_for_first_consumer(&pvc, &namespace, &name)
                .await?;
            return Ok(if accessibility_topologies.requeue {
                self.requeue_action()
            } else {
                Action::await_change()
            });
        }
        let reclaim_policy = reclaim_policy_from_storage_class(&storage_class)?;
        let access_modes = claim_access_modes(&namespace, &name, &spec.access_modes)?;
        let access_type = claim_access_type(&namespace, &name, spec.volume_mode.as_deref())?;
        let requested_capacity_bytes = requested_capacity_bytes(&namespace, &name, spec)?;
        let volume_content_source = match self
            .resolve_data_source(
                &pvc,
                &namespace,
                &name,
                &provisioner,
                &provisioner_config.socket_path,
                requested_capacity_bytes,
            )
            .await?
        {
            DataSourceResolution::Ready(source) => source,
            DataSourceResolution::Pending(action) => return Ok(action),
        };
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
            let provisioned_volume = match self
                .csi_operator
                .create_volume_with_source(
                    &provisioner_config.socket_path,
                    pv_name.clone(),
                    requested_capacity_bytes,
                    parameters,
                    access_modes.clone(),
                    access_type,
                    controller_create_secrets.clone(),
                    csi_config.mount_options.clone(),
                    accessibility_topologies.topologies,
                    volume_content_source.clone(),
                )
                .await
            {
                Ok(volume) => volume,
                Err(tugboat_csi_operator::Error::VolumeAlreadyExists) => {
                    let Some(existing) = pv_api.get(&pv_name).await? else {
                        tracing::warn!(
                            "CSI volume for PersistentVolume '{}' already exists but API object is not visible yet; requeueing",
                            pv_name
                        );
                        return Ok(self.requeue_action());
                    };
                    if !existing_pv_matches_claim(
                        &existing,
                        &namespace,
                        &name,
                        &storage_class_name,
                    )? {
                        return Err(ControllerError::ExistingVolumeConflict {
                            name: pv_name.clone(),
                            namespace: namespace.clone(),
                            claim: name.clone(),
                        });
                    }
                    // Another reconciler already provisioned this volume/PV pair.
                    return Ok(self.requeue_action());
                }
                Err(err) => return Err(err.into()),
            };
            let volume_id = provisioned_volume.volume_id.clone();
            provisioned_volume_id = Some(volume_id.clone());
            let pv_capacity_bytes = match (&volume_content_source, requested_capacity_bytes) {
                (Some(CsiVolumeContentSource::Snapshot { .. }), Some(requested))
                    if provisioned_volume.capacity_bytes < requested =>
                {
                    requested
                }
                _ => provisioned_volume.capacity_bytes,
            };

            let persistent_volume = build_persistent_volume(
                &pv_name,
                &namespace,
                &name,
                storage_class_name.clone(),
                provisioner,
                reclaim_policy,
                spec.access_modes.clone(),
                spec.volume_mode.clone(),
                Some(pv_capacity_bytes),
                csi_config.clone(),
                volume_id.clone(),
                provisioned_volume.volume_context.clone(),
                provisioned_volume.accessible_topology.clone(),
            );

            match pv_api.create(persistent_volume).await {
                Ok(_) => {
                    created_pv = true;
                }
                Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                    let Some(existing) = pv_api.get(&pv_name).await? else {
                        if !self
                            .cleanup_csi_volume_with_retry(
                                &provisioner_config.socket_path,
                                &volume_id,
                                &controller_create_secrets,
                            )
                            .await
                        {
                            return Err(ControllerError::ProvisioningCleanupFailed {
                                volume_id: volume_id.clone(),
                            });
                        }
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
                        if !self
                            .cleanup_csi_volume_with_retry(
                                &provisioner_config.socket_path,
                                &volume_id,
                                &controller_create_secrets,
                            )
                            .await
                        {
                            return Err(ControllerError::ProvisioningCleanupFailed {
                                volume_id: volume_id.clone(),
                            });
                        }
                        return Err(ControllerError::ExistingVolumeConflict {
                            name: pv_name.clone(),
                            namespace: namespace.clone(),
                            claim: name.clone(),
                        });
                    }
                }
                Err(err) => {
                    if !self
                        .cleanup_csi_volume_with_retry(
                            &provisioner_config.socket_path,
                            &volume_id,
                            &controller_create_secrets,
                        )
                        .await
                    {
                        return Err(ControllerError::ProvisioningCleanupFailed {
                            volume_id: volume_id.clone(),
                        });
                    }
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
                .await?;
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
                .await?;
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
                    .await?;
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

    async fn resolve_data_source(
        &self,
        pvc: &PersistentVolumeClaim,
        namespace: &str,
        name: &str,
        target_driver: &str,
        socket_path: &str,
        requested_capacity_bytes: Option<i64>,
    ) -> Result<DataSourceResolution, ControllerError> {
        let Some(data_source) = pvc.spec.as_ref().and_then(|spec| spec.data_source.as_ref()) else {
            return Ok(DataSourceResolution::Ready(None));
        };

        match data_source.kind.as_str() {
            "VolumeSnapshot" => {
                if !self
                    .controller_supports(socket_path, ControllerCapability::CreateDeleteSnapshot)
                    .await?
                {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "SnapshotNotSupported",
                        "CSI driver does not advertise CREATE_DELETE_SNAPSHOT; snapshot restore is not supported.",
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                }
                let snapshot_api: Api<VolumeSnapshot> =
                    Api::namespaced(self.client.clone(), namespace);
                let Some(snapshot) = snapshot_api.get(&data_source.name).await? else {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "WaitingForSnapshot",
                        &format!(
                            "VolumeSnapshot '{}' is not available yet.",
                            data_source.name
                        ),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(self.requeue_action()));
                };
                let Some(snapshot_status) = snapshot.status.as_ref() else {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "WaitingForSnapshot",
                        &format!("VolumeSnapshot '{}' is not ready yet.", data_source.name),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                };
                if snapshot_status.ready_to_use != Some(true) {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "WaitingForSnapshot",
                        &format!("VolumeSnapshot '{}' is not ready yet.", data_source.name),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                }
                if let (Some(requested), Some(restore_size)) = (
                    requested_capacity_bytes,
                    snapshot_status
                        .restore_size_bytes
                        .filter(|value| *value > 0),
                ) && requested < restore_size
                {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "SnapshotRestoreSizeExceeded",
                        &format!(
                            "Requested capacity {requested} is smaller than snapshot restore size {restore_size}."
                        ),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                }
                let Some(content_name) = snapshot_status
                    .bound_volume_snapshot_content_name
                    .as_deref()
                    .filter(|value| !value.is_empty())
                else {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "WaitingForSnapshot",
                        &format!(
                            "VolumeSnapshot '{}' is ready but is not bound to content yet.",
                            data_source.name
                        ),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                };
                let content_api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                let Some(content) = content_api.get(content_name).await? else {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "WaitingForSnapshot",
                        &format!("VolumeSnapshotContent '{content_name}' is not available yet."),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(self.requeue_action()));
                };
                let content_spec = content.spec.as_ref().ok_or_else(|| {
                    ControllerError::MissingVolumeSnapshotContentSpec {
                        name: content_name.to_string(),
                    }
                })?;
                if content_spec.driver != target_driver {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "SnapshotDriverMismatch",
                        &format!(
                            "VolumeSnapshotContent '{content_name}' uses driver '{}' but target StorageClass uses driver '{}'.",
                            content_spec.driver, target_driver
                        ),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                }
                let Some(snapshot_id) = content
                    .status
                    .as_ref()
                    .and_then(|status| status.snapshot_handle.as_deref())
                    .filter(|value| !value.is_empty())
                else {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "WaitingForSnapshot",
                        &format!(
                            "VolumeSnapshotContent '{content_name}' has no snapshot handle yet."
                        ),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                };
                Ok(DataSourceResolution::Ready(Some(
                    CsiVolumeContentSource::Snapshot {
                        snapshot_id: snapshot_id.to_string(),
                    },
                )))
            }
            "PersistentVolumeClaim" => {
                if !self
                    .controller_supports(socket_path, ControllerCapability::CloneVolume)
                    .await?
                {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "CloneNotSupported",
                        "CSI driver does not advertise CLONE_VOLUME; PVC clone is not supported.",
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                }
                let (source_driver, volume_id) = match self
                    .resolve_source_claim_volume(namespace, &data_source.name)
                    .await
                {
                    Ok(resolved) => resolved,
                    Err(ControllerError::SnapshotSourceUnavailable { reason, .. }) => {
                        self.apply_data_source_condition(
                            pvc,
                            namespace,
                            name,
                            "WaitingForSourcePVC",
                            &format!(
                                "PersistentVolumeClaim '{}' cannot be cloned yet: {reason}.",
                                data_source.name
                            ),
                        )
                        .await?;
                        return Ok(DataSourceResolution::Pending(Action::await_change()));
                    }
                    Err(err) => return Err(err),
                };
                if source_driver != target_driver {
                    self.apply_data_source_condition(
                        pvc,
                        namespace,
                        name,
                        "CloneSourceDriverMismatch",
                        &format!(
                            "Source PersistentVolumeClaim '{}' uses driver '{}' but target StorageClass uses driver '{}'.",
                            data_source.name, source_driver, target_driver
                        ),
                    )
                    .await?;
                    return Ok(DataSourceResolution::Pending(Action::await_change()));
                }
                Ok(DataSourceResolution::Ready(Some(
                    CsiVolumeContentSource::Volume { volume_id },
                )))
            }
            _ => Ok(DataSourceResolution::Ready(None)),
        }
    }

    async fn controller_supports(
        &self,
        socket_path: &str,
        capability: ControllerCapability,
    ) -> Result<bool, ControllerError> {
        Ok(self
            .csi_operator
            .controller_capabilities(socket_path)
            .await?
            .contains(&capability))
    }

    async fn apply_data_source_condition(
        &self,
        pvc: &PersistentVolumeClaim,
        namespace: &str,
        name: &str,
        condition_type: &str,
        message: &str,
    ) -> Result<(), ControllerError> {
        let mut updated = pvc.clone();
        let changed = apply_pvc_status(&mut updated, "Pending", None, false)
            | upsert_pvc_condition(&mut updated, condition_type, message);
        if changed {
            let pvc_api: Api<PersistentVolumeClaim> =
                Api::namespaced(self.client.clone(), namespace);
            pvc_api.replace_status(name, updated).await?;
        }
        Ok(())
    }

    async fn resolve_source_claim_volume(
        &self,
        namespace: &str,
        source_claim_name: &str,
    ) -> Result<(String, String), ControllerError> {
        let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(source_pvc) = pvc_api.get(source_claim_name).await? else {
            return Err(ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: source_claim_name.to_string(),
                reason: "source PersistentVolumeClaim is not available".to_string(),
            });
        };
        if source_pvc
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
            != Some("Bound")
        {
            return Err(ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: source_claim_name.to_string(),
                reason: "source PersistentVolumeClaim is not Bound".to_string(),
            });
        }
        let pv_name = source_pvc
            .spec
            .as_ref()
            .and_then(|spec| spec.volume_name.as_deref())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: source_claim_name.to_string(),
                reason: "source PersistentVolumeClaim has no bound volumeName".to_string(),
            })?
            .to_string();
        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(source_pv) = pv_api.get(&pv_name).await? else {
            return Err(ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: source_claim_name.to_string(),
                reason: format!("source PersistentVolume '{pv_name}' is not available"),
            });
        };
        let pv_spec = source_pv.spec.as_ref().ok_or_else(|| {
            ControllerError::MissingPersistentVolumeSpec {
                name: pv_name.clone(),
            }
        })?;
        let csi =
            pv_spec
                .csi
                .as_ref()
                .ok_or_else(|| ControllerError::MissingPersistentVolumeCsi {
                    name: pv_name.clone(),
                })?;
        if csi.volume_handle.is_empty() {
            return Err(ControllerError::MissingVolumeHandle { name: pv_name });
        }
        Ok((csi.driver.clone(), csi.volume_handle.clone()))
    }

    async fn cleanup_orphaned_volume(
        &self,
        socket_path: &str,
        volume_id: Option<&str>,
        volume_delete_secrets: &std::collections::HashMap<String, String>,
        pv_api: &Api<PersistentVolume>,
        pv_name: &str,
    ) -> Result<(), ControllerError> {
        if let Some(volume_id) = volume_id {
            let deleted = self
                .cleanup_csi_volume_with_retry(socket_path, volume_id, volume_delete_secrets)
                .await;
            if !deleted {
                return Err(ControllerError::ProvisioningCleanupFailed {
                    volume_id: volume_id.to_string(),
                });
            }
        }
        pv_api.delete(pv_name).await?;
        Ok(())
    }

    /// Attempts to delete a CSI volume with up to 3 retries on failure.
    async fn cleanup_csi_volume_with_retry(
        &self,
        socket_path: &str,
        volume_id: &str,
        secrets: &std::collections::HashMap<String, String>,
    ) -> bool {
        const MAX_RETRIES: usize = 3;
        const BASE_RETRY_DELAY: Duration = Duration::from_millis(200);
        for attempt in 1..=MAX_RETRIES {
            match self
                .csi_operator
                .delete_volume(socket_path, volume_id.to_string(), secrets.clone())
                .await
            {
                Ok(()) | Err(tugboat_csi_operator::Error::VolumeNotFound) => return true,
                Err(err) if attempt < MAX_RETRIES && is_retryable_csi_cleanup_error(&err) => {
                    tracing::warn!(
                        "Failed to clean up orphaned CSI volume '{}' (attempt {}/{}): {}",
                        volume_id,
                        attempt,
                        MAX_RETRIES,
                        err
                    );
                    let delay = BASE_RETRY_DELAY.saturating_mul(2u32.pow((attempt - 1) as u32));
                    sleep(delay).await;
                }
                Err(err) => {
                    if is_retryable_csi_cleanup_error(&err) {
                        tracing::error!(
                            "Failed to clean up orphaned CSI volume '{}' after {} attempts: {}; \
                             manual intervention may be required",
                            volume_id,
                            attempt,
                            err
                        );
                    } else {
                        tracing::error!(
                            "Failed to clean up orphaned CSI volume '{}' due to non-retryable \
                             error on attempt {}/{}: {}; manual intervention may be required",
                            volume_id,
                            attempt,
                            MAX_RETRIES,
                            err
                        );
                    }
                    return false;
                }
            }
        }
        false
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

fn upsert_pvc_condition(
    pvc: &mut PersistentVolumeClaim,
    condition_type: &str,
    message: &str,
) -> bool {
    let status = pvc
        .status
        .get_or_insert_with(PersistentVolumeClaimStatus::default);
    if let Some(existing) = status
        .conditions
        .iter_mut()
        .find(|condition| condition.status == condition_type)
    {
        if existing.message == message {
            return false;
        }
        existing.message = message.to_string();
        existing.timestamp = Some(Time::now());
        return true;
    }
    status.conditions.push(PersistentVolumeClaimCondition {
        status: condition_type.to_string(),
        message: message.to_string(),
        timestamp: Some(Time::now()),
    });
    true
}

fn selected_topology_labels(
    labels: &HashMap<String, String>,
    storage_class: &StorageClass,
) -> HashMap<String, String> {
    let allowed_keys = storage_class
        .spec
        .as_ref()
        .map(|spec| {
            spec.allowed_topologies
                .iter()
                .flat_map(|term| term.match_label_expressions.iter())
                .map(|requirement| requirement.key.clone())
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();

    if allowed_keys.is_empty() {
        return labels
            .iter()
            .filter(|(key, _)| key.starts_with("topology."))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
    }

    allowed_keys
        .into_iter()
        .filter_map(|key| labels.get(&key).map(|value| (key, value.clone())))
        .collect()
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
