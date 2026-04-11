pub(super) mod rs_ops;
pub(super) mod template_hash;
#[cfg(test)]
mod tests;

use crate::base::TugboatController;
use crate::change_classifier::{TemplateChangeKind, classify_template_change};
use crate::error::ControllerError;
use rs_ops::{
    REPLICASET_UPDATE_STRATEGY_ALL, active_replicaset, build_deployment_status, build_replicaset,
    build_replicaset_with_replicas, has_old_running_replicas, is_owned_by_deployment,
    managed_replicasets, next_rolling_update_rotation_targets, old_replicasets,
    replicaset_ready_replicas, replicaset_spec_replicas, replicaset_status_replicas,
    revision_history_limit, set_in_place_update_strategy, stale_replicasets_for_cleanup,
    update_replicaset_for_in_place, update_replicaset_replicas,
};
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;
use template_hash::{
    deployment_template, deployment_template_hash, replicaset_has_template_hash, template_hash,
};
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::apps::v1::{
    Deployment, ReplicaSet, ReplicaSetSpec, ShipTemplateSpec,
};
use tugboat_resources::manifests::core::v1::RuntimeClass;
use tugboat_resources::manifests::meta::v1::ObjectMeta;

#[derive(Clone)]
struct DeploymentReconciler {
    client: TugboatClient,
}

pub(crate) struct DeploymentController {
    controller: Controller<Deployment>,
    reconciler: DeploymentReconciler,
}

impl DeploymentController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: DeploymentReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for DeploymentController {
    fn name(&self) -> &str {
        "deployment"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

impl DeploymentReconciler {
    async fn resolve_runtime_class(
        &self,
        template: &ShipTemplateSpec,
    ) -> Result<Option<RuntimeClass>, ControllerError> {
        let runtime_class_name = template
            .spec
            .as_ref()
            .and_then(|spec| spec.runtime_class.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty());

        let Some(runtime_class_name) = runtime_class_name else {
            return Ok(None);
        };

        let runtime_class_api: Api<RuntimeClass> = Api::all(self.client.clone());
        runtime_class_api
            .get(runtime_class_name)
            .await
            .map_err(Into::into)
    }

    async fn reconcile_deleted(&self, dep: Deployment) -> Result<Action, ControllerError> {
        let namespace = dep.namespace().unwrap_or_default().to_string();
        let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), &namespace);
        let replicasets = rs_api.list().await?;

        for rs in replicasets
            .into_iter()
            .filter(|rs| is_owned_by_deployment(rs, &dep))
        {
            let name = rs.name().unwrap_or_default().to_string();
            rs_api.delete(&name).await?;
        }

        Ok(Action::await_change())
    }

    async fn ensure_rotation_replicaset(
        &self,
        dep: &Deployment,
        rs_api: &Api<ReplicaSet>,
        managed_replicasets: &[&ReplicaSet],
        initial_replicas: Option<i32>,
    ) -> Result<Option<ReplicaSet>, ControllerError> {
        let dep_template = deployment_template(dep)?;
        let hash = template_hash(dep_template);

        if let Some(existing_rs) = managed_replicasets
            .iter()
            .copied()
            .find(|rs| replicaset_has_template_hash(rs, &hash))
        {
            return Ok(Some(existing_rs.clone()));
        }

        self.create_replicaset_if_absent(
            rs_api,
            build_replicaset_with_replicas(dep, &hash, initial_replicas),
        )
        .await?;
        Ok(None)
    }

    async fn sync_deployment_status(
        &self,
        namespace: &str,
        dep: &Deployment,
        managed: &[&ReplicaSet],
    ) -> Result<(), ControllerError> {
        let active = active_replicaset(managed);
        let new_status = build_deployment_status(managed, active);

        if dep.status.as_ref() == Some(&new_status) {
            return Ok(());
        }

        let mut updated = dep.clone();
        updated.status = Some(new_status);
        let dep_api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        dep_api
            .patch_status(
                dep.name().unwrap_or_default(),
                json!({
                    "status": updated.status
                }),
            )
            .await?;
        Ok(())
    }

    async fn create_replicaset_if_absent(
        &self,
        rs_api: &Api<ReplicaSet>,
        rs: ReplicaSet,
    ) -> Result<(), ControllerError> {
        match rs_api.create(rs).await {
            Ok(_) => Ok(()),
            Err(tugboat_client::Error::Api(status)) if status.code == 409 => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    async fn replace_replicaset_if_changed(
        &self,
        rs_api: &Api<ReplicaSet>,
        current: &ReplicaSet,
        updated: ReplicaSet,
    ) -> Result<bool, ControllerError> {
        if &updated == current {
            return Ok(false);
        }

        match rs_api
            .replace(current.name().unwrap_or_default(), updated.clone())
            .await
        {
            Ok(_) => Ok(true),
            Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                let Some(name) = current.name() else {
                    return Ok(false);
                };
                let Some(mut latest) = rs_api.get(name).await? else {
                    return Ok(false);
                };

                if latest.spec == updated.spec
                    && latest.object_meta.as_ref().map(|meta| &meta.annotations)
                        == updated.object_meta.as_ref().map(|meta| &meta.annotations)
                {
                    return Ok(false);
                }

                latest.spec = updated.spec;
                latest.type_meta = updated.type_meta;

                if let Some(updated_meta) = updated.object_meta {
                    let latest_meta = latest.object_meta.get_or_insert_with(ObjectMeta::default);
                    latest_meta.labels = updated_meta.labels;
                    latest_meta.annotations = updated_meta.annotations;
                    latest_meta.owner_references = updated_meta.owner_references;
                }

                rs_api.replace(name, latest).await?;
                Ok(true)
            }
            Err(err) => Err(err.into()),
        }
    }

    async fn reconcile_recreate(
        &self,
        dep: &Deployment,
        rs_api: &Api<ReplicaSet>,
        managed_replicasets: &[&ReplicaSet],
        change_kind: TemplateChangeKind,
    ) -> Result<Action, ControllerError> {
        match change_kind {
            TemplateChangeKind::InPlace | TemplateChangeKind::Hotplug => {
                let active_rs = active_replicaset(managed_replicasets);
                let Some(active_rs) = active_rs else {
                    let hash = deployment_template_hash(dep)?;
                    self.create_replicaset_if_absent(rs_api, build_replicaset(dep, &hash))
                        .await?;
                    return Ok(Action::await_change());
                };

                let mut updated_rs = update_replicaset_for_in_place(active_rs, dep);
                set_in_place_update_strategy(&mut updated_rs, Some(REPLICASET_UPDATE_STRATEGY_ALL));
                self.replace_replicaset_if_changed(rs_api, active_rs, updated_rs)
                    .await?;
                Ok(Action::await_change())
            }
            TemplateChangeKind::NoChange => {
                let active_rs = active_replicaset(managed_replicasets);
                let old_replicasets =
                    old_replicasets(managed_replicasets, active_rs.and_then(|rs| rs.name()));
                let dep_spec = dep.spec.as_ref().cloned().unwrap_or_default();
                let desired = dep_spec.replicas.unwrap_or(1);

                if has_old_running_replicas(&old_replicasets) {
                    let mut changed = false;
                    for rs in old_replicasets {
                        let mut updated_rs = (*rs).clone();
                        updated_rs
                            .spec
                            .get_or_insert_with(ReplicaSetSpec::default)
                            .replicas = Some(0);
                        set_in_place_update_strategy(&mut updated_rs, None);
                        changed |= self
                            .replace_replicaset_if_changed(rs_api, rs, updated_rs)
                            .await?;
                    }
                    return if changed {
                        Ok(Action::requeue(Duration::from_secs(2)))
                    } else {
                        Ok(Action::await_change())
                    };
                }

                if let Some(active_rs) = active_rs {
                    let mut updated_rs = active_rs.clone();
                    updated_rs
                        .spec
                        .get_or_insert_with(ReplicaSetSpec::default)
                        .replicas = Some(desired);
                    set_in_place_update_strategy(&mut updated_rs, None);
                    if self
                        .replace_replicaset_if_changed(rs_api, active_rs, updated_rs)
                        .await?
                    {
                        return Ok(Action::requeue(Duration::from_secs(2)));
                    }
                }

                Ok(Action::await_change())
            }
            TemplateChangeKind::RequiresRotation => {
                let Some(active_rs) = self
                    .ensure_rotation_replicaset(dep, rs_api, managed_replicasets, Some(0))
                    .await?
                else {
                    return Ok(Action::requeue(Duration::from_secs(2)));
                };
                let old_replicasets = old_replicasets(managed_replicasets, active_rs.name());
                let desired = dep
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.replicas)
                    .unwrap_or(1);

                if has_old_running_replicas(&old_replicasets) {
                    let mut changed = false;
                    for rs in old_replicasets {
                        let mut updated_rs = rs.clone();
                        updated_rs
                            .spec
                            .get_or_insert_with(ReplicaSetSpec::default)
                            .replicas = Some(0);
                        set_in_place_update_strategy(&mut updated_rs, None);
                        changed |= self
                            .replace_replicaset_if_changed(rs_api, rs, updated_rs)
                            .await?;
                    }
                    return if changed {
                        Ok(Action::requeue(Duration::from_secs(2)))
                    } else {
                        Ok(Action::await_change())
                    };
                }

                if let Some(mut updated_rs) = update_replicaset_replicas(&active_rs, Some(desired))
                {
                    set_in_place_update_strategy(&mut updated_rs, None);
                    if self
                        .replace_replicaset_if_changed(rs_api, &active_rs, updated_rs)
                        .await?
                    {
                        return Ok(Action::requeue(Duration::from_secs(2)));
                    }
                }

                Ok(Action::await_change())
            }
        }
    }

    async fn reconcile_rolling_update(
        &self,
        dep: &Deployment,
        rs_api: &Api<ReplicaSet>,
        active_rs: Option<&ReplicaSet>,
        previous_replicasets: &[&ReplicaSet],
        change_kind: TemplateChangeKind,
    ) -> Result<Action, ControllerError> {
        let dep_spec = dep
            .spec
            .as_ref()
            .ok_or(ControllerError::MissingDeploymentSpec {
                namespace: dep.namespace().unwrap_or_default().to_string(),
                name: dep.name().unwrap_or_default().to_string(),
            })?;
        match change_kind {
            TemplateChangeKind::NoChange => {
                if let Some(active_rs) = active_rs {
                    if let Some(mut updated_rs) =
                        update_replicaset_replicas(active_rs, dep_spec.replicas)
                    {
                        set_in_place_update_strategy(&mut updated_rs, None);
                        self.replace_replicaset_if_changed(rs_api, active_rs, updated_rs)
                            .await?;
                    }
                } else {
                    let hash = deployment_template_hash(dep)?;
                    self.create_replicaset_if_absent(rs_api, build_replicaset(dep, &hash))
                        .await?;
                    return Ok(Action::requeue(Duration::from_secs(2)));
                }
            }
            TemplateChangeKind::InPlace | TemplateChangeKind::Hotplug => {
                if let Some(active_rs) = active_rs {
                    let mut updated_rs = update_replicaset_for_in_place(active_rs, dep);
                    set_in_place_update_strategy(&mut updated_rs, None);
                    if self
                        .replace_replicaset_if_changed(rs_api, active_rs, updated_rs)
                        .await?
                    {
                        return Ok(Action::requeue(Duration::from_secs(2)));
                    }

                    let desired = dep_spec.replicas.unwrap_or(1).max(0);
                    let ready = replicaset_ready_replicas(active_rs);
                    if ready < desired {
                        return Ok(Action::requeue(Duration::from_secs(5)));
                    }
                } else {
                    let hash = deployment_template_hash(dep)?;
                    self.create_replicaset_if_absent(rs_api, build_replicaset(dep, &hash))
                        .await?;
                    return Ok(Action::requeue(Duration::from_secs(2)));
                }
            }
            TemplateChangeKind::RequiresRotation => {
                let managed_replicasets: Vec<&ReplicaSet> = active_rs
                    .into_iter()
                    .chain(previous_replicasets.iter().copied())
                    .collect();
                // When no existing ReplicaSets are managed (fresh deployment), create the
                // initial RS with the desired replica count directly. For a rotation from an
                // existing RS, start at 0 and scale up incrementally via the rolling logic.
                let initial_replicas = if managed_replicasets.is_empty() {
                    dep_spec.replicas
                } else {
                    Some(0)
                };
                let Some(active_rs) = self
                    .ensure_rotation_replicaset(dep, rs_api, &managed_replicasets, initial_replicas)
                    .await?
                else {
                    return Ok(Action::requeue(Duration::from_secs(2)));
                };
                let rotation_old_replicasets =
                    old_replicasets(&managed_replicasets, active_rs.name());

                let (new_target, old_targets, should_requeue) =
                    next_rolling_update_rotation_targets(
                        dep,
                        &active_rs,
                        &rotation_old_replicasets,
                    );

                if let Some(target) = new_target
                    && let Some(mut updated_rs) =
                        update_replicaset_replicas(&active_rs, Some(target))
                {
                    set_in_place_update_strategy(&mut updated_rs, None);
                    self.replace_replicaset_if_changed(rs_api, &active_rs, updated_rs)
                        .await?;
                    return Ok(Action::requeue(Duration::from_secs(2)));
                }

                for old_rs in &rotation_old_replicasets {
                    let Some((_, target)) = old_targets
                        .iter()
                        .find(|(name, _)| old_rs.name().unwrap_or_default() == name.as_str())
                    else {
                        continue;
                    };

                    if let Some(mut updated_rs) = update_replicaset_replicas(old_rs, Some(*target))
                    {
                        set_in_place_update_strategy(&mut updated_rs, None);
                        self.replace_replicaset_if_changed(rs_api, old_rs, updated_rs)
                            .await?;
                        return Ok(Action::requeue(Duration::from_secs(2)));
                    }
                }

                if should_requeue {
                    return Ok(Action::requeue(Duration::from_secs(2)));
                }
            }
        }

        Ok(Action::await_change())
    }

    async fn reconcile_applied(&self, dep: Deployment) -> Result<Action, ControllerError> {
        let Some(namespace) = dep.namespace() else {
            return Err(ControllerError::MissingNamespace("Deployment"));
        };
        let dep_name = dep.name().unwrap_or_default().to_string();
        let dep_spec = dep
            .spec
            .as_ref()
            .ok_or(ControllerError::MissingDeploymentSpec {
                namespace: namespace.to_string(),
                name: dep_name,
            })?;
        let dep_template = deployment_template(&dep)?.clone();
        let runtime_class = self.resolve_runtime_class(&dep_template).await?;

        let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
        let replicasets = rs_api.list().await?;
        let managed = managed_replicasets(&replicasets, &dep);
        let active_rs = active_replicaset(&managed);
        let old_replicasets = old_replicasets(&managed, active_rs.and_then(|rs| rs.name()));
        let stale_replicasets =
            stale_replicasets_for_cleanup(&old_replicasets, revision_history_limit(&dep));

        let mut change_kind = active_rs
            .map(|active_rs| {
                classify_template_change(
                    &active_rs
                        .spec
                        .as_ref()
                        .and_then(|spec| spec.ship_template.clone())
                        .unwrap_or_default(),
                    &dep_template,
                    runtime_class.as_ref(),
                )
            })
            .unwrap_or(TemplateChangeKind::RequiresRotation);

        if change_kind == TemplateChangeKind::NoChange
            && old_replicasets
                .iter()
                .any(|rs| replicaset_spec_replicas(rs) > 0 || replicaset_status_replicas(rs) > 0)
        {
            change_kind = TemplateChangeKind::RequiresRotation;
        }

        let strategy_type = dep_spec
            .strategy
            .as_ref()
            .map(|strategy| strategy.r#type.as_str())
            .unwrap_or("RollingUpdate");

        let action = match strategy_type {
            "Recreate" => {
                self.reconcile_recreate(&dep, &rs_api, &managed, change_kind)
                    .await
            }
            _ => {
                self.reconcile_rolling_update(
                    &dep,
                    &rs_api,
                    active_rs,
                    &old_replicasets,
                    change_kind,
                )
                .await
            }
        }?;

        let stale_replicaset_names: HashSet<&str> = stale_replicasets
            .iter()
            .filter_map(|rs| rs.name())
            .collect();
        for rs in stale_replicasets {
            let name = rs.name().unwrap_or_default().to_string();
            rs_api.delete(&name).await?;
        }

        let managed_for_status: Vec<&ReplicaSet> = managed
            .iter()
            .copied()
            .filter(|rs| {
                rs.name()
                    .map(|name| !stale_replicaset_names.contains(name))
                    .unwrap_or(true)
            })
            .collect();
        self.sync_deployment_status(namespace, &dep, &managed_for_status)
            .await?;

        Ok(action)
    }
}

#[async_trait::async_trait]
impl Reconciler<Deployment> for DeploymentReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<Deployment>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(deployment) => self.reconcile_applied(deployment).await,
            ReconcileEvent::Deleted(deployment) => self.reconcile_deleted(deployment).await,
        }
    }
}
