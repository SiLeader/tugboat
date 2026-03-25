use crate::base::TugboatController;
use crate::config::ControllerManagerConfig;
use crate::error::ControllerError;
use crate::provisioning::{
    PV_FINALIZER, is_managed_pv, managed_pv_label_selector, provisioner_config, pv_provisioner,
    should_delete_backing_volume,
};
use tugboat_client::runtime::{
    Action, Controller, FinalizerEvent, ReconcileEvent, Reconciler, finalizer,
};
use tugboat_client::{Api, TugboatClient, WatchParams};
use tugboat_csi_operator::TugboatCsiOperator;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{PersistentVolume, PersistentVolumeClaim};

#[derive(Clone)]
struct PersistentVolumeCleanupReconciler {
    client: TugboatClient,
    csi_operator: TugboatCsiOperator,
    config: ControllerManagerConfig,
}

pub(crate) struct PersistentVolumeCleanupController {
    controller: Controller<PersistentVolume>,
    reconciler: PersistentVolumeCleanupReconciler,
}

impl PersistentVolumeCleanupController {
    pub(crate) fn new(
        client: TugboatClient,
        csi_operator: TugboatCsiOperator,
        config: ControllerManagerConfig,
    ) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone()))
                .with_watch_params(WatchParams::default().labels(managed_pv_label_selector())),
            reconciler: PersistentVolumeCleanupReconciler {
                client,
                csi_operator,
                config,
            },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for PersistentVolumeCleanupController {
    fn name(&self) -> &str {
        "pv-cleanup"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<PersistentVolume> for PersistentVolumeCleanupReconciler {
    type Error = ControllerError;

    async fn reconcile(
        &self,
        event: ReconcileEvent<PersistentVolume>,
    ) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(volume) => self.reconcile_applied(volume).await,
            ReconcileEvent::Deleted(_) => Ok(Action::await_change()),
        }
    }
}

impl PersistentVolumeCleanupReconciler {
    fn requeue_action(&self) -> Action {
        Action::requeue(self.config.csi.requeue_interval())
    }

    async fn reconcile_applied(
        &self,
        persistent_volume: PersistentVolume,
    ) -> Result<Action, ControllerError> {
        if !is_managed_pv(&persistent_volume) {
            return Ok(Action::await_change());
        }

        if !should_delete_backing_volume(&persistent_volume)? {
            return self.remove_finalizer_if_present(persistent_volume).await;
        }

        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        finalizer(&api, PV_FINALIZER, persistent_volume, {
            let this = self.clone();
            move |event| async move {
                match event {
                    FinalizerEvent::Apply(persistent_volume) => {
                        this.reconcile_active_volume(persistent_volume).await
                    }
                    FinalizerEvent::Cleanup(persistent_volume) => {
                        this.cleanup_backing_volume(persistent_volume).await
                    }
                }
            }
        })
        .await
        .map_err(|err| ControllerError::Finalizer(err.to_string()))
    }

    async fn reconcile_active_volume(
        &self,
        persistent_volume: PersistentVolume,
    ) -> Result<Action, ControllerError> {
        if self.bound_claim_exists(&persistent_volume).await? {
            return Ok(self.requeue_action());
        }

        let name = persistent_volume
            .name()
            .ok_or(ControllerError::MissingName("PersistentVolume"))?
            .to_string();
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        api.delete(&name).await?;
        Ok(Action::requeue_immediately())
    }

    async fn cleanup_backing_volume(
        &self,
        persistent_volume: PersistentVolume,
    ) -> Result<Action, ControllerError> {
        let name = persistent_volume
            .name()
            .ok_or(ControllerError::MissingName("PersistentVolume"))?
            .to_string();

        let provisioner = pv_provisioner(&persistent_volume)?;
        let Some(provisioner_config) = provisioner_config(&self.config, &provisioner) else {
            tracing::warn!(
                "Provisioner '{}' for managed PersistentVolume '{}' is not configured; skipping CSI volume cleanup",
                provisioner,
                name
            );
            return Ok(Action::await_change());
        };

        let spec = persistent_volume
            .spec
            .as_ref()
            .ok_or_else(|| ControllerError::MissingPersistentVolumeSpec { name: name.clone() })?;
        let csi = spec
            .csi
            .as_ref()
            .ok_or_else(|| ControllerError::MissingPersistentVolumeCsi { name: name.clone() })?;
        if csi.volume_handle.is_empty() {
            return Err(ControllerError::MissingVolumeHandle { name });
        }

        self.csi_operator
            .delete_volume(&provisioner_config.socket_path, csi.volume_handle.clone())
            .await?;
        Ok(Action::await_change())
    }

    async fn bound_claim_exists(
        &self,
        persistent_volume: &PersistentVolume,
    ) -> Result<bool, ControllerError> {
        let name = persistent_volume
            .name()
            .ok_or(ControllerError::MissingName("PersistentVolume"))?
            .to_string();
        let spec = persistent_volume
            .spec
            .as_ref()
            .ok_or_else(|| ControllerError::MissingPersistentVolumeSpec { name: name.clone() })?;
        let Some(claim_ref) = spec.claim_ref.as_ref() else {
            return Ok(false);
        };

        let claim_api: Api<PersistentVolumeClaim> =
            Api::namespaced(self.client.clone(), &claim_ref.namespace);
        Ok(claim_api.get(&claim_ref.name).await?.is_some())
    }

    async fn remove_finalizer_if_present(
        &self,
        persistent_volume: PersistentVolume,
    ) -> Result<Action, ControllerError> {
        if !persistent_volume.has_finalizer(PV_FINALIZER) {
            return Ok(Action::await_change());
        }

        let name = persistent_volume
            .name()
            .ok_or(ControllerError::MissingName("PersistentVolume"))?
            .to_string();
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        let mut updated = persistent_volume;
        updated.remove_finalizer(PV_FINALIZER);
        api.replace(&name, updated).await?;
        Ok(Action::await_change())
    }
}
