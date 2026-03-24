use crate::base::TugboatController;
use crate::config::ControllerManagerConfig;
use crate::error::ControllerError;
use crate::provisioning::{
    build_persistent_volume, claim_access_modes, claim_access_type, dynamic_volume_name,
    existing_pv_matches_claim, provisioner_config, pvc_identity, reclaim_policy_from_storage_class,
    storage_class_provisioner,
};
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_csi_operator::TugboatCsiOperator;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    PersistentVolume, PersistentVolumeClaim, StorageClass,
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

        if spec
            .volume_name
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            return Ok(Action::await_change());
        }

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
        let Some(provisioner_config) = provisioner_config(&self.config, &provisioner) else {
            return Ok(Action::await_change());
        };
        let reclaim_policy = reclaim_policy_from_storage_class(&storage_class)?;
        let access_modes = claim_access_modes(&namespace, &name, &spec.access_modes)?;
        let access_type = claim_access_type(&namespace, &name, spec.volume_mode.as_deref())?;
        let pv_name = dynamic_volume_name(&namespace, &name, uid.as_deref());

        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let mut created_pv = false;
        if let Some(existing) = pv_api.get(&pv_name).await? {
            if existing_pv_matches_claim(&existing, &namespace, &name, &storage_class_name)? {
                // PV already exists and matches this claim; skip volume creation.
            } else {
                return Err(ControllerError::ExistingVolumeConflict {
                    name: pv_name.clone(),
                    namespace: namespace.clone(),
                    claim: name.clone(),
                });
            }
        } else {
            let provisioned_volume = self
                .csi_operator
                .create_volume(
                    &provisioner_config.socket_path,
                    pv_name.clone(),
                    parameters,
                    access_modes,
                    access_type,
                )
                .await?;
            let provisioned_volume_id = provisioned_volume.volume_id.clone();

            let persistent_volume = build_persistent_volume(
                &pv_name,
                &namespace,
                &name,
                storage_class_name.clone(),
                provisioner,
                reclaim_policy,
                spec.access_modes.clone(),
                spec.volume_mode.clone(),
                provisioned_volume_id.clone(),
            );

            match pv_api.create(persistent_volume).await {
                Ok(_) => {
                    created_pv = true;
                }
                Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                    let Some(existing) = pv_api.get(&pv_name).await? else {
                        if let Err(cleanup_err) = self
                            .csi_operator
                            .delete_volume(
                                &provisioner_config.socket_path,
                                provisioned_volume_id.clone(),
                            )
                            .await
                        {
                            tracing::warn!("Failed to clean up orphaned volume: {}", cleanup_err);
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
                        if let Err(cleanup_err) = self
                            .csi_operator
                            .delete_volume(
                                &provisioner_config.socket_path,
                                provisioned_volume_id.clone(),
                            )
                            .await
                        {
                            tracing::warn!("Failed to clean up orphaned volume: {}", cleanup_err);
                        }
                        return Err(ControllerError::ExistingVolumeConflict {
                            name: pv_name.clone(),
                            namespace: namespace.clone(),
                            claim: name.clone(),
                        });
                    }
                }
                Err(err) => {
                    if let Err(cleanup_err) = self
                        .csi_operator
                        .delete_volume(&provisioner_config.socket_path, provisioned_volume_id)
                        .await
                    {
                        tracing::warn!("Failed to clean up orphaned volume: {}", cleanup_err);
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
                if let Err(cleanup_err) = pv_api.delete(&pv_name).await {
                    tracing::warn!(
                        "Failed to clean up orphaned PersistentVolume '{}': {}",
                        pv_name,
                        cleanup_err
                    );
                }
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
                if let Err(cleanup_err) = pv_api.delete(&pv_name).await {
                    tracing::warn!(
                        "Failed to clean up orphaned PersistentVolume '{}': {}",
                        pv_name,
                        cleanup_err
                    );
                }
            }
            return Ok(Action::await_change());
        }

        let latest_spec = latest.spec.as_mut().ok_or_else(|| {
            ControllerError::MissingPersistentVolumeClaimSpec {
                namespace: namespace.clone(),
                name: name.clone(),
            }
        })?;
        if let Some(existing_volume_name) = latest_spec
            .volume_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            if existing_volume_name == pv_name.as_str() {
                return Ok(Action::await_change());
            }
            if created_pv {
                tracing::warn!(
                    "PersistentVolumeClaim '{}/{}' is already bound to '{}'; cleaning up orphaned PersistentVolume '{}'",
                    namespace,
                    name,
                    existing_volume_name,
                    pv_name
                );
                if let Err(cleanup_err) = pv_api.delete(&pv_name).await {
                    tracing::warn!(
                        "Failed to clean up orphaned PersistentVolume '{}': {}",
                        pv_name,
                        cleanup_err
                    );
                }
            }
            return Ok(Action::await_change());
        }

        latest_spec.volume_name = Some(pv_name);
        pvc_api.replace(&name, latest).await?;
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
}
