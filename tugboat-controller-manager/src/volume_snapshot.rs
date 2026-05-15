use crate::base::TugboatController;
use crate::config::ControllerManagerConfig;
use crate::error::ControllerError;
use crate::provisioning::{
    is_retryable_csi_cleanup_error, load_secret_reference, provisioner_config,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use tugboat_client::runtime::{
    Action, Controller, FinalizerEvent, ReconcileEvent, Reconciler, finalizer,
};
use tugboat_client::{Api, TugboatClient};
use tugboat_csi_operator::TugboatCsiOperator;
use tugboat_resources::manifests::core::v1::{PersistentVolume, PersistentVolumeClaim};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference, Time};
use tugboat_resources::manifests::snapshot::v1::{
    VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotCondition, VolumeSnapshotContent,
    VolumeSnapshotContentSource, VolumeSnapshotContentSpec, VolumeSnapshotContentStatus,
    VolumeSnapshotRef, VolumeSnapshotStatus,
};
use tugboat_resources::{ObjectMetaResource, Resource};

pub(crate) const SNAPSHOT_FINALIZER: &str = "snapshot.tugboat.cloud/protection";

#[derive(Clone)]
struct VolumeSnapshotReconciler {
    client: TugboatClient,
    csi_operator: TugboatCsiOperator,
    config: ControllerManagerConfig,
}

pub(crate) struct VolumeSnapshotController {
    snapshot_controller: Controller<VolumeSnapshot>,
    content_controller: Controller<VolumeSnapshotContent>,
    reconciler: VolumeSnapshotReconciler,
}

impl VolumeSnapshotController {
    pub(crate) fn new(
        client: TugboatClient,
        csi_operator: TugboatCsiOperator,
        config: ControllerManagerConfig,
    ) -> Self {
        Self {
            snapshot_controller: Controller::new(Api::all(client.clone())),
            content_controller: Controller::new(Api::all(client.clone())),
            reconciler: VolumeSnapshotReconciler {
                client,
                csi_operator,
                config,
            },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for VolumeSnapshotController {
    fn name(&self) -> &str {
        "volume-snapshot"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        let snapshots = self
            .snapshot_controller
            .clone()
            .run(SnapshotEventReconciler(self.reconciler.clone()));
        let contents = self
            .content_controller
            .clone()
            .run(SnapshotContentEventReconciler(self.reconciler.clone()));
        tokio::join!(snapshots, contents);
    }
}

#[derive(Clone)]
struct SnapshotEventReconciler(VolumeSnapshotReconciler);

#[derive(Clone)]
struct SnapshotContentEventReconciler(VolumeSnapshotReconciler);

#[async_trait::async_trait]
impl Reconciler<VolumeSnapshot> for SnapshotEventReconciler {
    type Error = ControllerError;

    async fn reconcile(
        &self,
        event: ReconcileEvent<VolumeSnapshot>,
    ) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(snapshot) => self.0.reconcile_snapshot(snapshot).await,
            ReconcileEvent::Deleted(snapshot) => self.0.reconcile_deleted_snapshot(snapshot).await,
        }
    }
}

#[async_trait::async_trait]
impl Reconciler<VolumeSnapshotContent> for SnapshotContentEventReconciler {
    type Error = ControllerError;

    async fn reconcile(
        &self,
        event: ReconcileEvent<VolumeSnapshotContent>,
    ) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(content) => self.0.reconcile_content(content).await,
            ReconcileEvent::Deleted(_) => Ok(Action::await_change()),
        }
    }
}

impl VolumeSnapshotReconciler {
    fn requeue_action(&self) -> Action {
        Action::requeue(self.config.csi.requeue_interval())
    }

    async fn reconcile_snapshot(
        &self,
        snapshot: VolumeSnapshot,
    ) -> Result<Action, ControllerError> {
        if snapshot.deletion_timestamp().is_some() {
            return Ok(Action::await_change());
        }

        let (namespace, name, uid) = snapshot_identity(&snapshot)?;
        let bound_content_name = snapshot
            .status
            .as_ref()
            .and_then(|status| status.bound_volume_snapshot_content_name.as_deref())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        if let Some(content_name) = bound_content_name {
            self.propagate_content_to_snapshot(&namespace, &name, &content_name)
                .await?;
            return Ok(Action::await_change());
        }

        let spec =
            snapshot
                .spec
                .clone()
                .ok_or_else(|| ControllerError::MissingVolumeSnapshotSpec {
                    namespace: namespace.clone(),
                    name: name.clone(),
                })?;
        let source =
            spec.source
                .as_ref()
                .ok_or_else(|| ControllerError::MissingVolumeSnapshotSource {
                    namespace: namespace.clone(),
                    name: name.clone(),
                })?;

        if let Some(content_name) = source
            .volume_snapshot_content_name
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        {
            self.bind_snapshot_to_content(snapshot, &namespace, &name, &content_name)
                .await?;
            return Ok(Action::await_change());
        }

        let Some(pvc_name) = source
            .persistent_volume_claim_name
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        else {
            return Ok(Action::await_change());
        };
        let Some(class_name) = spec
            .volume_snapshot_class_name
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        else {
            self.apply_snapshot_condition(
                snapshot,
                &namespace,
                &name,
                "SnapshotClassMissing",
                "A dynamic VolumeSnapshot requires spec.volumeSnapshotClassName.",
            )
            .await?;
            return Ok(Action::await_change());
        };

        let class_api: Api<VolumeSnapshotClass> = Api::all(self.client.clone());
        let Some(snapshot_class) = class_api.get(&class_name).await? else {
            self.apply_snapshot_condition(
                snapshot,
                &namespace,
                &name,
                "SnapshotClassNotFound",
                &format!("VolumeSnapshotClass '{class_name}' is not available yet."),
            )
            .await?;
            return Ok(self.requeue_action());
        };
        let class_spec = snapshot_class.spec.as_ref().ok_or_else(|| {
            ControllerError::MissingVolumeSnapshotClassSpec {
                name: class_name.clone(),
            }
        })?;

        let (pv_name, driver, volume_handle) =
            self.resolve_source_pvc(&namespace, &pvc_name).await?;
        if class_spec.driver != driver {
            self.apply_snapshot_condition(
                snapshot,
                &namespace,
                &name,
                "SnapshotClassDriverMismatch",
                &format!(
                    "VolumeSnapshotClass '{class_name}' uses driver '{}' but source PersistentVolume '{}' uses driver '{}'.",
                    class_spec.driver, pv_name, driver
                ),
            )
            .await?;
            return Ok(Action::await_change());
        }

        let content_name = deterministic_content_name(uid.as_deref(), &namespace, &name);
        let snapshot_ref = VolumeSnapshotRef {
            name: name.clone(),
            namespace: namespace.clone(),
            uid: uid.clone().unwrap_or_default(),
        };
        let content = build_snapshot_content(
            &content_name,
            snapshot_ref,
            &class_name,
            class_spec.deletion_policy.clone(),
            driver,
            volume_handle,
        );
        let content_api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
        match content_api.create(content).await {
            Ok(_) => {}
            Err(tugboat_client::Error::Api(status)) if status.code == 409 => {}
            Err(err) => return Err(err.into()),
        }

        self.bind_snapshot_to_content(snapshot, &namespace, &name, &content_name)
            .await?;
        Ok(Action::requeue_immediately())
    }

    async fn reconcile_deleted_snapshot(
        &self,
        snapshot: VolumeSnapshot,
    ) -> Result<Action, ControllerError> {
        let (namespace, name, uid) = snapshot_identity(&snapshot)?;
        let content_api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());

        // Optimize: try deterministic name if UID is available
        if let Some(uid_str) = uid.as_deref() {
            let content_name = deterministic_content_name(Some(uid_str), &namespace, &name);
            match content_api.delete(&content_name).await {
                Ok(_) => return Ok(Action::await_change()),
                Err(tugboat_client::Error::Api(status)) if status.code == 404 => {}
                Err(err) => return Err(err.into()),
            }
        }

        // Fallback: list and match if deterministic name didn't work or UID was missing
        for content in content_api.list().await? {
            let Some(spec) = content.spec.as_ref() else {
                continue;
            };
            let Some(reference) = spec.volume_snapshot_ref.as_ref() else {
                continue;
            };
            if reference.namespace == namespace
                && reference.name == name
                && (uid.is_none() || Some(reference.uid.as_str()) == uid.as_deref())
                && let Some(content_name) = content.name()
            {
                content_api.delete(content_name).await?;
            }
        }
        Ok(Action::await_change())
    }

    async fn reconcile_content(
        &self,
        content: VolumeSnapshotContent,
    ) -> Result<Action, ControllerError> {
        let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
        finalizer(&api, SNAPSHOT_FINALIZER, content, {
            let this = self.clone();
            move |event| async move {
                match event {
                    FinalizerEvent::Apply(content) => this.reconcile_active_content(content).await,
                    FinalizerEvent::Cleanup(content) => {
                        this.cleanup_snapshot_content(content).await
                    }
                }
            }
        })
        .await
        .map_err(|err| ControllerError::Finalizer(err.to_string()))
    }

    async fn reconcile_active_content(
        &self,
        mut content: VolumeSnapshotContent,
    ) -> Result<Action, ControllerError> {
        let content_name = content
            .name()
            .ok_or(ControllerError::MissingName("VolumeSnapshotContent"))?
            .to_string();
        if let Some(status) = content.status.as_ref()
            && let Some(snapshot_handle) = status
                .snapshot_handle
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        {
            if status.ready_to_use != Some(true) {
                return self
                    .refresh_existing_snapshot(content, &content_name, &snapshot_handle)
                    .await;
            }
            self.propagate_bound_content(&content).await?;
            return Ok(Action::await_change());
        }

        let spec = content.spec.clone().ok_or_else(|| {
            ControllerError::MissingVolumeSnapshotContentSpec {
                name: content_name.clone(),
            }
        })?;
        let source = spec.source.as_ref().ok_or_else(|| {
            ControllerError::MissingVolumeSnapshotContentSource {
                name: content_name.clone(),
            }
        })?;

        if let Some(snapshot_handle) = source
            .snapshot_handle
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        {
            patch_content_status(
                &mut content,
                Some(snapshot_handle),
                Some(Time::now()),
                Some(true),
                None,
                None,
            );
            let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
            api.replace_status(&content_name, content.clone()).await?;
            self.propagate_bound_content(&content).await?;
            return Ok(Action::await_change());
        }

        let Some(volume_handle) = source
            .volume_handle
            .as_deref()
            .or(spec.source_volume_handle.as_deref())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        else {
            patch_content_status(&mut content, None, None, Some(false), None, Some(
                "VolumeSnapshotContent requires spec.source.volumeHandle for dynamic snapshot creation."
                    .to_string(),
            ));
            let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
            api.replace_status(&content_name, content).await?;
            return Ok(Action::await_change());
        };

        let Some(provisioner_config) = provisioner_config(&self.config, &spec.driver) else {
            patch_content_status(
                &mut content,
                None,
                None,
                Some(false),
                None,
                Some(format!(
                    "CSI provisioner '{}' is not configured for snapshot creation.",
                    spec.driver
                )),
            );
            let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
            api.replace_status(&content_name, content).await?;
            return Ok(Action::await_change());
        };

        let (parameters, secret_ref) = if let Some(class_name) = spec
            .volume_snapshot_class_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let class_api: Api<VolumeSnapshotClass> = Api::all(self.client.clone());
            let Some(class) = class_api.get(class_name).await? else {
                patch_content_status(
                    &mut content,
                    None,
                    None,
                    Some(false),
                    None,
                    Some(format!(
                        "VolumeSnapshotClass '{class_name}' is not available yet."
                    )),
                );
                let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                api.replace_status(&content_name, content).await?;
                return Ok(self.requeue_action());
            };
            let class_spec = class.spec.as_ref().ok_or_else(|| {
                ControllerError::MissingVolumeSnapshotClassSpec {
                    name: class_name.to_string(),
                }
            })?;
            (
                class_spec.parameters.clone(),
                class_spec.snapshotter_secret_ref.clone(),
            )
        } else {
            (HashMap::new(), None)
        };
        let secrets = load_secret_reference(&self.client, secret_ref.as_ref()).await?;

        match self
            .csi_operator
            .create_snapshot(
                &provisioner_config.socket_path,
                volume_handle,
                content_name.clone(),
                parameters,
                secrets,
            )
            .await
        {
            Ok(snapshot) => {
                patch_content_status(
                    &mut content,
                    Some(snapshot.snapshot_id),
                    Some(Time {
                        seconds: snapshot.creation_time_seconds,
                        nanos: snapshot.creation_time_nanos,
                    }),
                    Some(snapshot.ready_to_use),
                    snapshot.size_bytes,
                    None,
                );
                let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                api.replace_status(&content_name, content.clone()).await?;
                self.propagate_bound_content(&content).await?;
                Ok(Action::await_change())
            }
            Err(err) => {
                let retryable = is_retryable_csi_cleanup_error(&err);
                patch_content_status(
                    &mut content,
                    None,
                    None,
                    Some(false),
                    None,
                    Some(err.to_string()),
                );
                let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                api.replace_status(&content_name, content).await?;
                Ok(if retryable {
                    self.requeue_action()
                } else {
                    Action::await_change()
                })
            }
        }
    }

    async fn cleanup_snapshot_content(
        &self,
        content: VolumeSnapshotContent,
    ) -> Result<Action, ControllerError> {
        let content_name = content
            .name()
            .ok_or(ControllerError::MissingName("VolumeSnapshotContent"))?
            .to_string();
        let spec = content.spec.as_ref().ok_or_else(|| {
            ControllerError::MissingVolumeSnapshotContentSpec {
                name: content_name.clone(),
            }
        })?;
        if spec.deletion_policy != "Delete" {
            return Ok(Action::await_change());
        }

        let snapshot_handle = content
            .status
            .as_ref()
            .and_then(|status| status.snapshot_handle.as_deref())
            .or_else(|| {
                spec.source
                    .as_ref()
                    .and_then(|source| source.snapshot_handle.as_deref())
            })
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let Some(snapshot_handle) = snapshot_handle else {
            return Ok(Action::await_change());
        };
        let Some(provisioner_config) = provisioner_config(&self.config, &spec.driver) else {
            tracing::warn!(
                "Provisioner '{}' for VolumeSnapshotContent '{}' is not configured; skipping CSI snapshot cleanup",
                spec.driver,
                content_name
            );
            return Ok(Action::await_change());
        };
        let secrets = if let Some(class_name) = spec
            .volume_snapshot_class_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let class_api: Api<VolumeSnapshotClass> = Api::all(self.client.clone());
            match class_api.get(class_name).await? {
                Some(class) => {
                    let class_spec = class.spec.as_ref().ok_or_else(|| {
                        ControllerError::MissingVolumeSnapshotClassSpec {
                            name: class_name.to_string(),
                        }
                    })?;
                    load_secret_reference(&self.client, class_spec.snapshotter_secret_ref.as_ref())
                        .await?
                }
                None => HashMap::new(),
            }
        } else {
            HashMap::new()
        };

        self.delete_snapshot_with_retry(
            &content_name,
            &provisioner_config.socket_path,
            snapshot_handle,
            secrets,
        )
        .await?;
        Ok(Action::await_change())
    }

    async fn refresh_existing_snapshot(
        &self,
        mut content: VolumeSnapshotContent,
        content_name: &str,
        snapshot_handle: &str,
    ) -> Result<Action, ControllerError> {
        let spec = content.spec.clone().ok_or_else(|| {
            ControllerError::MissingVolumeSnapshotContentSpec {
                name: content_name.to_string(),
            }
        })?;
        let Some(provisioner_config) = provisioner_config(&self.config, &spec.driver) else {
            self.propagate_bound_content(&content).await?;
            return Ok(self.requeue_action());
        };
        let capabilities = self
            .csi_operator
            .controller_capabilities(&provisioner_config.socket_path)
            .await?;
        if !capabilities.contains(&tugboat_csi_operator::ControllerCapability::ListSnapshots) {
            self.propagate_bound_content(&content).await?;
            return Ok(self.requeue_action());
        }
        let secrets = if let Some(class_name) = spec
            .volume_snapshot_class_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let class_api: Api<VolumeSnapshotClass> = Api::all(self.client.clone());
            match class_api.get(class_name).await? {
                Some(class) => {
                    let class_spec = class.spec.as_ref().ok_or_else(|| {
                        ControllerError::MissingVolumeSnapshotClassSpec {
                            name: class_name.to_string(),
                        }
                    })?;
                    load_secret_reference(&self.client, class_spec.snapshotter_secret_ref.as_ref())
                        .await?
                }
                None => HashMap::new(),
            }
        } else {
            HashMap::new()
        };

        match self
            .csi_operator
            .list_snapshots(
                &provisioner_config.socket_path,
                Some(snapshot_handle.to_string()),
                None,
                None,
                secrets,
            )
            .await
        {
            Ok(listed) => {
                if let Some(snapshot) = listed.entries.into_iter().next() {
                    let ready_to_use = snapshot.ready_to_use;
                    patch_content_status(
                        &mut content,
                        Some(snapshot.snapshot_id),
                        Some(Time {
                            seconds: snapshot.creation_time_seconds,
                            nanos: snapshot.creation_time_nanos,
                        }),
                        Some(ready_to_use),
                        snapshot.size_bytes,
                        None,
                    );
                    let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                    api.replace_status(content_name, content.clone()).await?;
                    self.propagate_bound_content(&content).await?;
                    Ok(if ready_to_use {
                        Action::await_change()
                    } else {
                        self.requeue_action()
                    })
                } else {
                    patch_content_status(
                        &mut content,
                        None,
                        None,
                        Some(false),
                        None,
                        Some(format!(
                            "CSI snapshot '{snapshot_handle}' was not returned by ListSnapshots."
                        )),
                    );
                    let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                    api.replace_status(content_name, content.clone()).await?;
                    self.propagate_bound_content(&content).await?;
                    Ok(Action::await_change())
                }
            }
            Err(err) => {
                let retryable = is_retryable_csi_cleanup_error(&err);
                patch_content_status(
                    &mut content,
                    None,
                    None,
                    Some(false),
                    None,
                    Some(err.to_string()),
                );
                let api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
                api.replace_status(content_name, content).await?;
                Ok(if retryable {
                    self.requeue_action()
                } else {
                    Action::await_change()
                })
            }
        }
    }

    async fn delete_snapshot_with_retry(
        &self,
        content_name: &str,
        socket_path: &str,
        snapshot_handle: String,
        secrets: HashMap<String, String>,
    ) -> Result<(), ControllerError> {
        const MAX_RETRIES: usize = 4;
        const BASE_RETRY_DELAY: Duration = Duration::from_millis(250);

        for attempt in 1..=MAX_RETRIES {
            match self
                .csi_operator
                .delete_snapshot(socket_path, snapshot_handle.clone(), secrets.clone())
                .await
            {
                Ok(()) | Err(tugboat_csi_operator::Error::SnapshotNotFound) => return Ok(()),
                Err(err) if attempt < MAX_RETRIES && is_retryable_csi_cleanup_error(&err) => {
                    tracing::warn!(
                        "Failed to delete CSI snapshot '{}' for VolumeSnapshotContent '{}' (attempt {}/{}): {}",
                        snapshot_handle,
                        content_name,
                        attempt,
                        MAX_RETRIES,
                        err
                    );
                    let delay = BASE_RETRY_DELAY.saturating_mul(2u32.pow((attempt - 1) as u32));
                    sleep(delay).await;
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    async fn resolve_source_pvc(
        &self,
        namespace: &str,
        pvc_name: &str,
    ) -> Result<(String, String, String), ControllerError> {
        let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(pvc) = pvc_api.get(pvc_name).await? else {
            return Err(ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: pvc_name.to_string(),
                reason: "source PersistentVolumeClaim is not available".to_string(),
            });
        };
        let pv_name = pvc
            .spec
            .as_ref()
            .and_then(|spec| spec.volume_name.as_deref())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: pvc_name.to_string(),
                reason: "source PersistentVolumeClaim is not bound".to_string(),
            })?
            .to_string();
        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(pv) = pv_api.get(&pv_name).await? else {
            return Err(ControllerError::SnapshotSourceUnavailable {
                namespace: namespace.to_string(),
                name: pvc_name.to_string(),
                reason: format!("source PersistentVolume '{pv_name}' is not available"),
            });
        };
        let spec =
            pv.spec
                .as_ref()
                .ok_or_else(|| ControllerError::MissingPersistentVolumeSpec {
                    name: pv_name.clone(),
                })?;
        let csi = spec
            .csi
            .as_ref()
            .ok_or_else(|| ControllerError::MissingPersistentVolumeCsi {
                name: pv_name.clone(),
            })?;
        if csi.volume_handle.is_empty() {
            return Err(ControllerError::MissingVolumeHandle { name: pv_name });
        }
        Ok((pv_name, csi.driver.clone(), csi.volume_handle.clone()))
    }

    async fn bind_snapshot_to_content(
        &self,
        mut snapshot: VolumeSnapshot,
        namespace: &str,
        name: &str,
        content_name: &str,
    ) -> Result<(), ControllerError> {
        let status = snapshot
            .status
            .get_or_insert_with(VolumeSnapshotStatus::default);
        status.bound_volume_snapshot_content_name = Some(content_name.to_string());
        let api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), namespace);
        api.replace_status(name, snapshot).await?;
        Ok(())
    }

    async fn apply_snapshot_condition(
        &self,
        mut snapshot: VolumeSnapshot,
        namespace: &str,
        name: &str,
        condition_type: &str,
        message: &str,
    ) -> Result<(), ControllerError> {
        let status = snapshot
            .status
            .get_or_insert_with(VolumeSnapshotStatus::default);
        upsert_snapshot_condition(status, condition_type, "False", message);
        let api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), namespace);
        api.replace_status(name, snapshot).await?;
        Ok(())
    }

    async fn propagate_content_to_snapshot(
        &self,
        namespace: &str,
        snapshot_name: &str,
        content_name: &str,
    ) -> Result<(), ControllerError> {
        let content_api: Api<VolumeSnapshotContent> = Api::all(self.client.clone());
        if let Some(content) = content_api.get(content_name).await? {
            self.propagate_bound_content(&content).await?;
        } else {
            let snapshot_api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), namespace);
            if let Some(mut snapshot) = snapshot_api.get(snapshot_name).await? {
                let status = snapshot
                    .status
                    .get_or_insert_with(VolumeSnapshotStatus::default);
                upsert_snapshot_condition(
                    status,
                    "SnapshotContentNotFound",
                    "False",
                    &format!("VolumeSnapshotContent '{content_name}' is not available yet."),
                );
                snapshot_api.replace_status(snapshot_name, snapshot).await?;
            }
        }
        Ok(())
    }

    async fn propagate_bound_content(
        &self,
        content: &VolumeSnapshotContent,
    ) -> Result<(), ControllerError> {
        let Some(spec) = content.spec.as_ref() else {
            return Ok(());
        };
        let Some(reference) = spec.volume_snapshot_ref.as_ref() else {
            return Ok(());
        };
        if reference.namespace.is_empty() || reference.name.is_empty() {
            return Ok(());
        }
        let api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), &reference.namespace);
        let Some(mut snapshot) = api.get(&reference.name).await? else {
            return Ok(());
        };
        let content_name = content.name().map(ToOwned::to_owned);
        let snapshot_status = snapshot
            .status
            .get_or_insert_with(VolumeSnapshotStatus::default);
        if snapshot_status.bound_volume_snapshot_content_name.is_none() {
            snapshot_status.bound_volume_snapshot_content_name = content_name;
        }
        if let Some(content_status) = content.status.as_ref() {
            snapshot_status.creation_time = content_status.creation_time;
            snapshot_status.ready_to_use = content_status.ready_to_use;
            snapshot_status.restore_size_bytes = content_status.restore_size_bytes;
            if let Some(error) = content_status.error.as_ref() {
                upsert_snapshot_condition(snapshot_status, "SnapshotError", "False", error);
            }
        }
        api.replace_status(&reference.name, snapshot).await?;
        Ok(())
    }
}

fn snapshot_identity(
    snapshot: &VolumeSnapshot,
) -> Result<(String, String, Option<String>), ControllerError> {
    let namespace = snapshot
        .namespace()
        .ok_or(ControllerError::MissingNamespace("VolumeSnapshot"))?
        .to_string();
    let name = snapshot
        .name()
        .ok_or(ControllerError::MissingName("VolumeSnapshot"))?
        .to_string();
    let uid = snapshot
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.uid.clone());
    Ok((namespace, name, uid))
}

fn build_snapshot_content(
    content_name: &str,
    snapshot_ref: VolumeSnapshotRef,
    class_name: &str,
    deletion_policy: String,
    driver: String,
    source_volume_handle: String,
) -> VolumeSnapshotContent {
    VolumeSnapshotContent {
        type_meta: Some(VolumeSnapshotContent::type_meta()),
        object_meta: Some(ObjectMeta {
            name: Some(content_name.to_string()),
            owner_references: vec![OwnerReference {
                api_version: "snapshot/v1".to_string(),
                kind: "VolumeSnapshot".to_string(),
                name: snapshot_ref.name.clone(),
                uid: snapshot_ref.uid.clone(),
                controller: Some(true),
            }],
            ..Default::default()
        }),
        spec: Some(VolumeSnapshotContentSpec {
            driver,
            deletion_policy,
            volume_snapshot_ref: Some(snapshot_ref),
            source: Some(VolumeSnapshotContentSource {
                volume_handle: Some(source_volume_handle.clone()),
                snapshot_handle: None,
            }),
            volume_snapshot_class_name: Some(class_name.to_string()),
            source_volume_handle: Some(source_volume_handle),
        }),
        status: None,
    }
}

fn patch_content_status(
    content: &mut VolumeSnapshotContent,
    snapshot_handle: Option<String>,
    creation_time: Option<Time>,
    ready_to_use: Option<bool>,
    restore_size_bytes: Option<i64>,
    error: Option<String>,
) {
    let status = content
        .status
        .get_or_insert_with(VolumeSnapshotContentStatus::default);
    if snapshot_handle.is_some() {
        status.snapshot_handle = snapshot_handle;
    }
    if creation_time.is_some() {
        status.creation_time = creation_time;
    }
    if ready_to_use.is_some() {
        status.ready_to_use = ready_to_use;
    }
    if restore_size_bytes.is_some() {
        status.restore_size_bytes = restore_size_bytes;
    }
    status.error = error;
}

fn upsert_snapshot_condition(
    status: &mut VolumeSnapshotStatus,
    condition_type: &str,
    condition_status: &str,
    message: &str,
) {
    if let Some(existing) = status
        .conditions
        .iter_mut()
        .find(|condition| condition.r#type == condition_type)
    {
        existing.status = condition_status.to_string();
        existing.message = message.to_string();
        existing.timestamp = Some(Time::now());
        return;
    }
    status.conditions.push(VolumeSnapshotCondition {
        r#type: condition_type.to_string(),
        status: condition_status.to_string(),
        message: message.to_string(),
        timestamp: Some(Time::now()),
    });
}

fn deterministic_content_name(uid: Option<&str>, namespace: &str, snapshot_name: &str) -> String {
    let source = uid
        .filter(|value| !value.is_empty())
        .map(|uid| format!("snapcontent-{uid}"))
        .unwrap_or_else(|| format!("snapcontent-{namespace}-{snapshot_name}"));
    sanitize_resource_name(&source, "snapcontent")
}

fn sanitize_resource_name(input: &str, fallback: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    let mut previous_was_dash = false;
    for ch in input.chars() {
        let lowered = ch.to_ascii_lowercase();
        if lowered.is_ascii_lowercase() || lowered.is_ascii_digit() {
            sanitized.push(lowered);
            previous_was_dash = false;
        } else if !previous_was_dash && !sanitized.is_empty() {
            sanitized.push('-');
            previous_was_dash = true;
        }
    }
    while sanitized.ends_with('-') {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push_str(fallback);
    }
    if sanitized.len() > 253 {
        sanitized.truncate(253);
        while sanitized.ends_with('-') {
            sanitized.pop();
        }
    }
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}
