use crate::base::TugboatController;
use crate::change_classifier::{TemplateChangeKind, classify_template_change};
use crate::error::ControllerError;
use serde_json::{Value, to_string};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::time::Duration;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::apps::v1::{
    Deployment, DeploymentStatus, ReplicaSet, ReplicaSetSpec, ShipTemplateSpec,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta};

const REPLICASET_UPDATE_STRATEGY_ANNOTATION: &str = "tugboat.dev/update-strategy";
const REPLICASET_UPDATE_STRATEGY_ALL: &str = "all";

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

fn owner_reference_for_deployment(dep: &Deployment) -> OwnerReference {
    OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "Deployment".to_string(),
        name: dep.name().unwrap_or_default().to_string(),
        uid: dep
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.as_deref())
            .unwrap_or_default()
            .to_string(),
        controller: Some(true),
    }
}

fn is_owned_by_deployment(rs: &ReplicaSet, dep: &Deployment) -> bool {
    let dep_name = dep.name().unwrap_or_default();
    let dep_uid = dep
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.uid.as_deref());

    rs.object_meta().as_ref().is_some_and(|meta| {
        meta.owner_references.iter().any(|owner_ref| {
            owner_ref.kind == "Deployment"
                && owner_ref.name == dep_name
                && dep_uid
                    .map(|uid| owner_ref.uid == uid)
                    .unwrap_or_else(|| owner_ref.uid.is_empty())
        })
    })
}

fn managed_replicasets<'a>(replicasets: &'a [ReplicaSet], dep: &Deployment) -> Vec<&'a ReplicaSet> {
    replicasets
        .iter()
        .filter(|rs| is_owned_by_deployment(rs, dep))
        .collect()
}

fn replicaset_creation_sort_key(rs: &ReplicaSet) -> (i64, i32, String) {
    let (seconds, nanos) = rs
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.creation_timestamp.as_ref())
        .map(|time| (time.seconds, time.nanos))
        .unwrap_or((0, 0));
    (seconds, nanos, rs.name().unwrap_or_default().to_string())
}

fn active_replicaset<'a>(replicasets: &'a [&ReplicaSet]) -> Option<&'a ReplicaSet> {
    replicasets
        .iter()
        .copied()
        .max_by_key(|rs| replicaset_creation_sort_key(rs))
}

fn template_hash(template: &ShipTemplateSpec) -> String {
    let json = canonical_json_string(template);
    let hash = Sha256::digest(json.as_bytes());
    hash[..4].iter().map(|byte| format!("{byte:02x}")).collect()
}

fn canonicalize_json_value(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            Value::Array(items.into_iter().map(canonicalize_json_value).collect())
        }
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json_value(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

fn canonical_json_string(template: &ShipTemplateSpec) -> String {
    serde_json::to_value(template)
        .map(canonicalize_json_value)
        .and_then(|value| to_string(&value))
        .unwrap_or_default()
}

fn replicaset_has_template_hash(rs: &ReplicaSet, hash: &str) -> bool {
    rs.object_meta()
        .as_ref()
        .and_then(|meta| meta.labels.get("ship-template-hash"))
        .is_some_and(|value| value == hash)
        || rs
            .spec
            .as_ref()
            .and_then(|spec| spec.selector.get("ship-template-hash"))
            .is_some_and(|value| value == hash)
}

fn build_replicaset_with_replicas(
    dep: &Deployment,
    hash: &str,
    replicas: Option<i32>,
) -> ReplicaSet {
    let dep_name = dep.name().unwrap_or_default();
    let dep_namespace = dep.namespace().map(str::to_string);
    let dep_spec = dep.spec.as_ref().cloned().unwrap_or_default();
    let mut selector = dep_spec.selector;
    selector.insert("ship-template-hash".to_string(), hash.to_string());

    let mut labels: HashMap<String, String> = selector.clone();
    labels.insert("ship-template-hash".to_string(), hash.to_string());

    let mut rs = ReplicaSet {
        object_meta: Some(ObjectMeta {
            name: Some(format!("{dep_name}-{hash}")),
            namespace: dep_namespace,
            labels,
            owner_references: vec![owner_reference_for_deployment(dep)],
            ..Default::default()
        }),
        spec: Some(ReplicaSetSpec {
            replicas,
            selector,
            ship_template: dep_spec.ship_template,
        }),
        status: None,
        ..Default::default()
    };
    rs.set_type_meta(ReplicaSet::type_meta());
    rs
}

fn build_replicaset(dep: &Deployment, hash: &str) -> ReplicaSet {
    let replicas = dep.spec.as_ref().and_then(|spec| spec.replicas);
    build_replicaset_with_replicas(dep, hash, replicas)
}

fn update_replicaset_for_in_place(active_rs: &ReplicaSet, dep: &Deployment) -> ReplicaSet {
    let mut updated_rs = active_rs.clone();
    let dep_spec = dep.spec.as_ref().cloned().unwrap_or_default();
    let rs_spec = updated_rs.spec.get_or_insert_with(ReplicaSetSpec::default);
    rs_spec.ship_template = dep_spec.ship_template;
    rs_spec.replicas = dep_spec.replicas;
    updated_rs
}

fn set_in_place_update_strategy(rs: &mut ReplicaSet, strategy: Option<&str>) {
    let metadata = rs.object_meta.get_or_insert_with(ObjectMeta::default);
    match strategy {
        Some(value) => {
            metadata.annotations.insert(
                REPLICASET_UPDATE_STRATEGY_ANNOTATION.to_string(),
                value.to_string(),
            );
        }
        None => {
            metadata
                .annotations
                .remove(REPLICASET_UPDATE_STRATEGY_ANNOTATION);
        }
    }
}

fn has_old_running_replicas(old_replicasets: &[&ReplicaSet]) -> bool {
    old_replicasets.iter().any(|rs| {
        let replicas = rs
            .status
            .as_ref()
            .map(|status| status.replicas)
            .or_else(|| rs.spec.as_ref().and_then(|spec| spec.replicas))
            .unwrap_or(0);
        replicas > 0
    })
}

fn old_replicasets<'a>(
    managed: &'a [&ReplicaSet],
    active_rs_name: Option<&str>,
) -> Vec<&'a ReplicaSet> {
    managed
        .iter()
        .copied()
        .filter(|rs| rs.name() != active_rs_name)
        .collect()
}

fn stale_replicasets_for_cleanup<'a>(
    old_replicasets: &'a [&ReplicaSet],
    revision_history_limit: usize,
) -> Vec<&'a ReplicaSet> {
    let mut zeroed: Vec<&ReplicaSet> = old_replicasets
        .iter()
        .copied()
        .filter(|rs| {
            let has_no_ships = rs
                .status
                .as_ref()
                .map(|status| status.replicas == 0)
                .unwrap_or(true);
            let is_scaled_to_zero = rs.spec.as_ref().and_then(|spec| spec.replicas) == Some(0);
            is_scaled_to_zero && has_no_ships
        })
        .collect();

    zeroed.sort_by_key(|rs| replicaset_creation_sort_key(rs));
    let keep = revision_history_limit.min(zeroed.len());
    let delete_count = zeroed.len() - keep;
    zeroed.into_iter().take(delete_count).collect()
}

fn update_replicaset_replicas(active_rs: &ReplicaSet, replicas: Option<i32>) -> Option<ReplicaSet> {
    let rs_spec = active_rs.spec.as_ref()?;
    if rs_spec.replicas == replicas {
        return None;
    }

    let mut updated_rs = active_rs.clone();
    updated_rs
        .spec
        .get_or_insert_with(ReplicaSetSpec::default)
        .replicas = replicas;
    Some(updated_rs)
}

fn replicaset_spec_replicas(rs: &ReplicaSet) -> i32 {
    rs.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(0)
}

fn replicaset_status_replicas(rs: &ReplicaSet) -> i32 {
    rs.status
        .as_ref()
        .map(|status| status.replicas)
        .unwrap_or_else(|| replicaset_spec_replicas(rs))
}

fn replicaset_ready_replicas(rs: &ReplicaSet) -> i32 {
    rs.status
        .as_ref()
        .map(|status| status.ready_replicas)
        .unwrap_or(0)
}

fn rolling_update_limits(dep: &Deployment) -> (i32, i32) {
    let strategy = dep.spec.as_ref().and_then(|spec| spec.strategy.as_ref());
    let rolling = strategy.and_then(|strategy| strategy.rolling_update.as_ref());
    let max_surge = rolling.and_then(|rolling| rolling.max_surge).unwrap_or(1);
    let max_unavailable = rolling
        .and_then(|rolling| rolling.max_unavailable)
        .unwrap_or(1);
    (max_surge.max(0), max_unavailable.max(0))
}

fn sum_status_replicas(replicasets: &[&ReplicaSet]) -> i32 {
    replicasets
        .iter()
        .map(|rs| replicaset_status_replicas(rs))
        .sum()
}

fn sum_ready_replicas(replicasets: &[&ReplicaSet]) -> i32 {
    replicasets
        .iter()
        .map(|rs| replicaset_ready_replicas(rs))
        .sum()
}

fn build_deployment_status(
    managed_replicasets: &[&ReplicaSet],
    active_replicaset: Option<&ReplicaSet>,
) -> DeploymentStatus {
    DeploymentStatus {
        replicas: sum_status_replicas(managed_replicasets),
        ready_replicas: sum_ready_replicas(managed_replicasets),
        updated_replicas: active_replicaset
            .map(replicaset_status_replicas)
            .unwrap_or(0),
    }
}

fn next_rolling_update_rotation_targets(
    dep: &Deployment,
    active_rs: &ReplicaSet,
    old_replicasets: &[&ReplicaSet],
) -> (Option<i32>, Vec<(String, i32)>, bool) {
    let desired = dep
        .spec
        .as_ref()
        .and_then(|spec| spec.replicas)
        .unwrap_or(1)
        .max(0);
    let (max_surge, max_unavailable) = rolling_update_limits(dep);
    let old_total = sum_status_replicas(old_replicasets);
    let old_ready = sum_ready_replicas(old_replicasets);
    let new_spec = replicaset_spec_replicas(active_rs);
    let new_total = replicaset_status_replicas(active_rs);
    let new_ready = replicaset_ready_replicas(active_rs);
    let total = old_total + new_total;

    let mut target_new_spec = None;
    let mut old_targets = Vec::new();
    let mut progressed = false;

    let can_add = (desired + max_surge) - total;
    if can_add > 0 && new_spec < desired {
        target_new_spec = Some((new_spec + can_add).min(desired));
        progressed = true;
    }

    let min_available = (desired - max_unavailable).max(0);
    let available = old_ready + new_ready;
    let mut can_remove = (available - min_available).max(0);
    if target_new_spec.is_some() {
        can_remove = 0;
    }

    for rs in old_replicasets {
        if can_remove <= 0 {
            break;
        }
        let current = replicaset_spec_replicas(rs);
        if current <= 0 {
            continue;
        }
        let decrement = current.min(can_remove);
        let target = current - decrement;
        if target != current {
            old_targets.push((rs.name().unwrap_or_default().to_string(), target));
            can_remove -= decrement;
            progressed = true;
        }
    }

    let rollout_complete = old_total == 0 && new_spec >= desired && new_ready >= desired;
    (
        target_new_spec,
        old_targets,
        progressed || !rollout_complete,
    )
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
        let dep_template = dep
            .spec
            .as_ref()
            .and_then(|spec| spec.ship_template.as_ref())
            .cloned()
            .unwrap_or_default();
        let hash = template_hash(&dep_template);

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
    ) -> Result<(), ControllerError> {
        let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
        let replicasets = rs_api.list().await?;
        let managed = managed_replicasets(&replicasets, dep);
        let active = active_replicaset(&managed);
        let new_status = build_deployment_status(&managed, active);

        if dep.status.as_ref() == Some(&new_status) {
            return Ok(());
        }

        let mut updated = dep.clone();
        updated.status = Some(new_status);
        let dep_api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        dep_api
            .replace(dep.name().unwrap_or_default(), updated)
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

        rs_api
            .replace(current.name().unwrap_or_default(), updated)
            .await?;
        Ok(true)
    }

    async fn reconcile_recreate(
        &self,
        dep: &Deployment,
        rs_api: &Api<ReplicaSet>,
        managed_replicasets: &[&ReplicaSet],
        change_kind: TemplateChangeKind,
    ) -> Result<Action, ControllerError> {
        match change_kind {
            TemplateChangeKind::InPlace => {
                let active_rs = active_replicaset(managed_replicasets);
                let Some(active_rs) = active_rs else {
                    let hash = dep
                        .spec
                        .as_ref()
                        .and_then(|spec| spec.ship_template.as_ref())
                        .map(template_hash)
                        .unwrap_or_default();
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
                    let hash = dep_spec
                        .ship_template
                        .as_ref()
                        .map(template_hash)
                        .unwrap_or_default();
                    self.create_replicaset_if_absent(rs_api, build_replicaset(dep, &hash))
                        .await?;
                    return Ok(Action::requeue(Duration::from_secs(2)));
                }
            }
            TemplateChangeKind::InPlace => {
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
                    let hash = dep_spec
                        .ship_template
                        .as_ref()
                        .map(template_hash)
                        .unwrap_or_default();
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
                let Some(active_rs) = self
                    .ensure_rotation_replicaset(dep, rs_api, &managed_replicasets, Some(0))
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
        let dep_template = dep_spec.ship_template.clone().unwrap_or_default();

        let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
        let replicasets = rs_api.list().await?;
        let managed = managed_replicasets(&replicasets, &dep);
        let active_rs = active_replicaset(&managed);
        let old_replicasets = old_replicasets(&managed, active_rs.and_then(|rs| rs.name()));
        let stale_replicasets = stale_replicasets_for_cleanup(&old_replicasets, 0);

        let change_kind = active_rs
            .map(|active_rs| {
                classify_template_change(
                    &active_rs
                        .spec
                        .as_ref()
                        .and_then(|spec| spec.ship_template.clone())
                        .unwrap_or_default(),
                    &dep_template,
                )
            })
            .unwrap_or(TemplateChangeKind::RequiresRotation);

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

        for rs in stale_replicasets {
            let name = rs.name().unwrap_or_default().to_string();
            rs_api.delete(&name).await?;
        }

        self.sync_deployment_status(namespace, &dep).await?;

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

#[cfg(test)]
mod tests {
    use super::{
        REPLICASET_UPDATE_STRATEGY_ALL, REPLICASET_UPDATE_STRATEGY_ANNOTATION, active_replicaset,
        build_deployment_status, build_replicaset, has_old_running_replicas,
        is_owned_by_deployment, managed_replicasets, next_rolling_update_rotation_targets,
        old_replicasets, owner_reference_for_deployment, replicaset_has_template_hash,
        replicaset_ready_replicas, rolling_update_limits, set_in_place_update_strategy,
        stale_replicasets_for_cleanup, template_hash, update_replicaset_for_in_place,
        update_replicaset_replicas,
    };
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::apps::v1::{
        Deployment, DeploymentSpec, DeploymentStatus, DeploymentStrategy, ReplicaSet,
        ReplicaSetSpec, ReplicaSetStatus, RollingUpdateStrategy, ShipTemplateSpec,
    };
    use tugboat_resources::manifests::core::v1::ShipSpec;
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference, Time};

    fn deployment() -> Deployment {
        Deployment {
            object_meta: Some(ObjectMeta {
                name: Some("demo".to_string()),
                namespace: Some("default".to_string()),
                uid: Some("dep-uid".to_string()),
                ..Default::default()
            }),
            spec: Some(DeploymentSpec {
                replicas: Some(3),
                selector: [("app".to_string(), "demo".to_string())]
                    .into_iter()
                    .collect(),
                ship_template: Some(ShipTemplateSpec {
                    spec: Some(ShipSpec {
                        image: "ghcr.io/example/demo:v1".to_string(),
                        ship_class: "standard".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn owned_replicaset(name: &str, seconds: i64) -> ReplicaSet {
        ReplicaSet {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                creation_timestamp: Some(Time { seconds, nanos: 0 }),
                owner_references: vec![OwnerReference {
                    api_version: "apps/v1".to_string(),
                    kind: "Deployment".to_string(),
                    name: "demo".to_string(),
                    uid: "dep-uid".to_string(),
                    controller: Some(true),
                }],
                ..Default::default()
            }),
            spec: Some(ReplicaSetSpec {
                replicas: Some(1),
                selector: [("app".to_string(), "demo".to_string())]
                    .into_iter()
                    .collect(),
                ship_template: Some(ShipTemplateSpec {
                    spec: Some(ShipSpec {
                        image: "ghcr.io/example/demo:v1".to_string(),
                        ship_class: "standard".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }
    }

    fn deployment_with_rolling_update(
        max_surge: Option<i32>,
        max_unavailable: Option<i32>,
    ) -> Deployment {
        let mut dep = deployment();
        dep.spec.as_mut().unwrap().strategy = Some(DeploymentStrategy {
            r#type: "RollingUpdate".to_string(),
            rolling_update: Some(RollingUpdateStrategy {
                max_surge,
                max_unavailable,
            }),
        });
        dep
    }

    #[test]
    fn owner_reference_matches_deployment() {
        let dep = deployment();
        assert_eq!(
            owner_reference_for_deployment(&dep),
            OwnerReference {
                api_version: "apps/v1".to_string(),
                kind: "Deployment".to_string(),
                name: "demo".to_string(),
                uid: "dep-uid".to_string(),
                controller: Some(true),
            }
        );
    }

    #[test]
    fn build_replicaset_sets_hash_on_selector_and_labels() {
        let dep = deployment();
        let hash = template_hash(
            dep.spec
                .as_ref()
                .and_then(|spec| spec.ship_template.as_ref())
                .unwrap(),
        );

        let rs = build_replicaset(&dep, &hash);
        let meta = rs.object_meta.as_ref().unwrap();
        let spec = rs.spec.as_ref().unwrap();

        assert_eq!(meta.name.as_deref(), Some(format!("demo-{hash}").as_str()));
        assert_eq!(
            meta.labels.get("ship-template-hash").map(String::as_str),
            Some(hash.as_str())
        );
        assert_eq!(
            spec.selector.get("ship-template-hash").map(String::as_str),
            Some(hash.as_str())
        );
    }

    #[test]
    fn build_replicaset_with_replicas_overrides_replica_count() {
        let dep = deployment();
        let hash = template_hash(
            dep.spec
                .as_ref()
                .and_then(|spec| spec.ship_template.as_ref())
                .unwrap(),
        );

        let rs = super::build_replicaset_with_replicas(&dep, &hash, Some(0));

        assert_eq!(rs.spec.as_ref().unwrap().replicas, Some(0));
    }

    #[test]
    fn template_hash_is_stable_for_same_template() {
        let dep = deployment();
        let template = dep
            .spec
            .as_ref()
            .and_then(|spec| spec.ship_template.as_ref())
            .unwrap();

        assert_eq!(template_hash(template), template_hash(template));
    }

    #[test]
    fn template_hash_is_stable_for_equivalent_map_orderings() {
        let template_a = ShipTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: [
                    ("a".to_string(), "1".to_string()),
                    ("b".to_string(), "2".to_string()),
                ]
                .into_iter()
                .collect(),
                annotations: [
                    ("x".to_string(), "1".to_string()),
                    ("y".to_string(), "2".to_string()),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                image: "ghcr.io/example/demo:v1".to_string(),
                ship_class: "standard".to_string(),
                ..Default::default()
            }),
        };
        let template_b = ShipTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: [
                    ("b".to_string(), "2".to_string()),
                    ("a".to_string(), "1".to_string()),
                ]
                .into_iter()
                .collect(),
                annotations: [
                    ("y".to_string(), "2".to_string()),
                    ("x".to_string(), "1".to_string()),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                image: "ghcr.io/example/demo:v1".to_string(),
                ship_class: "standard".to_string(),
                ..Default::default()
            }),
        };

        assert_eq!(template_hash(&template_a), template_hash(&template_b));
    }

    #[test]
    fn managed_replicasets_only_include_owned_sets() {
        let dep = deployment();
        let owned = owned_replicaset("demo-rs-1", 1);
        let mut foreign = owned_replicaset("other-rs", 2);
        foreign.object_meta.as_mut().unwrap().owner_references[0].uid = "other".to_string();
        let replicasets = [owned.clone(), foreign];

        let managed = managed_replicasets(&replicasets, &dep);

        assert_eq!(managed.len(), 1);
        assert!(is_owned_by_deployment(managed[0], &dep));
        assert_eq!(managed[0].name(), Some("demo-rs-1"));
    }

    #[test]
    fn active_replicaset_prefers_newest() {
        let older = owned_replicaset("demo-rs-1", 1);
        let newer = owned_replicaset("demo-rs-2", 2);

        assert_eq!(
            active_replicaset(&[&older, &newer])
                .and_then(|rs| rs.name())
                .unwrap(),
            "demo-rs-2"
        );
    }

    #[test]
    fn in_place_update_rewrites_template_and_replicas() {
        let mut dep = deployment();
        let dep_spec = dep.spec.as_mut().unwrap();
        dep_spec.replicas = Some(5);
        dep_spec
            .ship_template
            .as_mut()
            .unwrap()
            .spec
            .as_mut()
            .unwrap()
            .ship_class = "large".to_string();

        let rs = owned_replicaset("demo-rs", 1);
        let updated = update_replicaset_for_in_place(&rs, &dep);

        assert_eq!(updated.spec.as_ref().unwrap().replicas, Some(5));
        assert_eq!(
            updated
                .spec
                .as_ref()
                .and_then(|spec| spec.ship_template.as_ref())
                .and_then(|template| template.spec.as_ref())
                .map(|spec| spec.ship_class.as_str()),
            Some("large")
        );
    }

    #[test]
    fn replica_only_update_is_detected() {
        let rs = owned_replicaset("demo-rs", 1);
        let updated = update_replicaset_replicas(&rs, Some(4)).unwrap();

        assert_eq!(updated.spec.as_ref().unwrap().replicas, Some(4));
    }

    #[test]
    fn matching_hash_can_be_found_from_replicaset() {
        let dep = deployment();
        let hash = template_hash(
            dep.spec
                .as_ref()
                .and_then(|spec| spec.ship_template.as_ref())
                .unwrap(),
        );
        let rs = build_replicaset(&dep, &hash);

        assert!(replicaset_has_template_hash(&rs, &hash));
    }

    #[test]
    fn recreate_in_place_sets_all_update_strategy_annotation() {
        let rs = owned_replicaset("demo-rs", 1);
        let mut updated = update_replicaset_for_in_place(&rs, &deployment());

        set_in_place_update_strategy(&mut updated, Some(REPLICASET_UPDATE_STRATEGY_ALL));

        assert_eq!(
            updated
                .object_meta
                .as_ref()
                .and_then(|meta| meta.annotations.get(REPLICASET_UPDATE_STRATEGY_ANNOTATION))
                .map(String::as_str),
            Some(REPLICASET_UPDATE_STRATEGY_ALL)
        );
    }

    #[test]
    fn old_replicaset_list_excludes_active_set() {
        let older = owned_replicaset("demo-rs-1", 1);
        let newer = owned_replicaset("demo-rs-2", 2);
        let managed = vec![&older, &newer];

        let old = old_replicasets(&managed, Some("demo-rs-2"));

        assert_eq!(old.len(), 1);
        assert_eq!(old[0].name(), Some("demo-rs-1"));
    }

    #[test]
    fn old_running_replicas_detect_status_or_spec() {
        let mut with_status = owned_replicaset("demo-rs-1", 1);
        with_status.status = Some(ReplicaSetStatus {
            replicas: 2,
            ready_replicas: 0,
        });
        let mut with_zero_spec = owned_replicaset("demo-rs-2", 2);
        with_zero_spec.spec.as_mut().unwrap().replicas = Some(0);

        assert!(has_old_running_replicas(&[&with_status]));
        assert!(!has_old_running_replicas(&[&with_zero_spec]));
    }

    #[test]
    fn stale_replicasets_cleanup_selects_zeroed_sets() {
        let mut zeroed = owned_replicaset("demo-rs-old", 1);
        zeroed.spec.as_mut().unwrap().replicas = Some(0);
        zeroed.status = Some(ReplicaSetStatus {
            replicas: 0,
            ready_replicas: 0,
        });
        let mut still_running = owned_replicaset("demo-rs-running", 2);
        still_running.spec.as_mut().unwrap().replicas = Some(0);
        still_running.status = Some(ReplicaSetStatus {
            replicas: 1,
            ready_replicas: 0,
        });

        let stale = stale_replicasets_for_cleanup(&[&zeroed, &still_running], 0);

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].name(), Some("demo-rs-old"));
    }

    #[test]
    fn stale_replicasets_cleanup_keeps_newest_revision_history_limit() {
        let mut oldest = owned_replicaset("demo-rs-1", 1);
        oldest.spec.as_mut().unwrap().replicas = Some(0);
        oldest.status = Some(ReplicaSetStatus {
            replicas: 0,
            ready_replicas: 0,
        });
        let mut newer = owned_replicaset("demo-rs-2", 2);
        newer.spec.as_mut().unwrap().replicas = Some(0);
        newer.status = Some(ReplicaSetStatus {
            replicas: 0,
            ready_replicas: 0,
        });

        let stale = stale_replicasets_for_cleanup(&[&newer, &oldest], 1);

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].name(), Some("demo-rs-1"));
    }

    #[test]
    fn rolling_update_uses_default_limits() {
        let dep = deployment();

        assert_eq!(rolling_update_limits(&dep), (1, 1));
    }

    #[test]
    fn rolling_update_scales_up_new_replicaset_within_max_surge() {
        let dep = deployment_with_rolling_update(Some(1), Some(1));
        let mut active = owned_replicaset("demo-rs-new", 2);
        active.spec.as_mut().unwrap().replicas = Some(0);
        active.status = Some(ReplicaSetStatus {
            replicas: 0,
            ready_replicas: 0,
        });
        let mut old = owned_replicaset("demo-rs-old", 1);
        old.spec.as_mut().unwrap().replicas = Some(3);
        old.status = Some(ReplicaSetStatus {
            replicas: 3,
            ready_replicas: 3,
        });

        let (new_target, old_targets, should_requeue) =
            next_rolling_update_rotation_targets(&dep, &active, &[&old]);

        assert_eq!(new_target, Some(1));
        assert!(old_targets.is_empty());
        assert!(should_requeue);
    }

    #[test]
    fn rolling_update_scales_down_old_replicaset_when_availability_allows() {
        let dep = deployment_with_rolling_update(Some(1), Some(1));
        let mut active = owned_replicaset("demo-rs-new", 2);
        active.spec.as_mut().unwrap().replicas = Some(3);
        active.status = Some(ReplicaSetStatus {
            replicas: 3,
            ready_replicas: 3,
        });
        let mut old = owned_replicaset("demo-rs-old", 1);
        old.spec.as_mut().unwrap().replicas = Some(2);
        old.status = Some(ReplicaSetStatus {
            replicas: 2,
            ready_replicas: 2,
        });

        let (new_target, old_targets, should_requeue) =
            next_rolling_update_rotation_targets(&dep, &active, &[&old]);

        assert_eq!(new_target, None);
        assert_eq!(old_targets, vec![("demo-rs-old".to_string(), 0)]);
        assert!(should_requeue);
    }

    #[test]
    fn rolling_update_respects_zero_max_unavailable() {
        let dep = deployment_with_rolling_update(Some(1), Some(0));
        let mut active = owned_replicaset("demo-rs-new", 2);
        active.spec.as_mut().unwrap().replicas = Some(1);
        active.status = Some(ReplicaSetStatus {
            replicas: 1,
            ready_replicas: 1,
        });
        let mut old = owned_replicaset("demo-rs-old", 1);
        old.spec.as_mut().unwrap().replicas = Some(3);
        old.status = Some(ReplicaSetStatus {
            replicas: 3,
            ready_replicas: 2,
        });

        let (new_target, old_targets, should_requeue) =
            next_rolling_update_rotation_targets(&dep, &active, &[&old]);

        assert_eq!(new_target, None);
        assert!(old_targets.is_empty());
        assert!(should_requeue);
    }

    #[test]
    fn rolling_update_rotation_does_not_scale_new_replicaset_back_to_zero() {
        let dep = deployment_with_rolling_update(Some(1), Some(1));
        let mut active = owned_replicaset("demo-rs-new", 2);
        active.spec.as_mut().unwrap().replicas = Some(1);
        active.status = Some(ReplicaSetStatus {
            replicas: 1,
            ready_replicas: 1,
        });
        let mut old = owned_replicaset("demo-rs-old", 1);
        old.spec.as_mut().unwrap().replicas = Some(3);
        old.status = Some(ReplicaSetStatus {
            replicas: 3,
            ready_replicas: 3,
        });

        let (new_target, old_targets, should_requeue) =
            next_rolling_update_rotation_targets(&dep, &active, &[&old]);

        assert_eq!(new_target, None);
        assert_eq!(old_targets, vec![("demo-rs-old".to_string(), 1)]);
        assert!(should_requeue);
    }

    #[test]
    fn old_replicaset_lookup_uses_rotation_hash_match_not_newest_creation_time() {
        let dep = deployment();
        let current_hash = template_hash(
            dep.spec
                .as_ref()
                .and_then(|spec| spec.ship_template.as_ref())
                .unwrap(),
        );
        let mut stale = owned_replicaset("demo-rs-stale", 10);
        stale
            .object_meta
            .as_mut()
            .unwrap()
            .labels
            .insert("ship-template-hash".to_string(), "deadbeef".to_string());
        stale
            .spec
            .as_mut()
            .unwrap()
            .selector
            .insert("ship-template-hash".to_string(), "deadbeef".to_string());

        let mut matching_old = owned_replicaset("demo-rs-current", 1);
        matching_old
            .object_meta
            .as_mut()
            .unwrap()
            .labels
            .insert("ship-template-hash".to_string(), current_hash.clone());
        matching_old
            .spec
            .as_mut()
            .unwrap()
            .selector
            .insert("ship-template-hash".to_string(), current_hash.clone());

        let managed = vec![&stale, &matching_old];
        let active = active_replicaset(&managed).unwrap();
        let old = old_replicasets(&managed, active.name());

        let matched = old
            .iter()
            .copied()
            .find(|rs| replicaset_has_template_hash(rs, &current_hash))
            .unwrap();

        assert_eq!(active.name(), Some("demo-rs-stale"));
        assert_eq!(matched.name(), Some("demo-rs-current"));
    }

    #[test]
    fn ready_replicas_defaults_to_zero_without_status() {
        let rs = owned_replicaset("demo-rs", 1);

        assert_eq!(replicaset_ready_replicas(&rs), 0);
    }

    #[test]
    fn deployment_status_aggregates_all_managed_replicasets() {
        let mut active = owned_replicaset("demo-rs-new", 2);
        active.status = Some(ReplicaSetStatus {
            replicas: 3,
            ready_replicas: 2,
        });
        let mut old = owned_replicaset("demo-rs-old", 1);
        old.status = Some(ReplicaSetStatus {
            replicas: 2,
            ready_replicas: 1,
        });

        let status = build_deployment_status(&[&active, &old], Some(&active));

        assert_eq!(
            status,
            DeploymentStatus {
                replicas: 5,
                ready_replicas: 3,
                updated_replicas: 3,
            }
        );
    }

    #[test]
    fn deployment_status_defaults_updated_replicas_to_zero_without_active_replicaset() {
        let status = build_deployment_status(&[], None);

        assert_eq!(
            status,
            DeploymentStatus {
                replicas: 0,
                ready_replicas: 0,
                updated_replicas: 0,
            }
        );
    }
}
