use crate::workload::{SHIP_TEMPLATE_HASH_LABEL, is_controlled_by, owner_reference_for};
use std::collections::HashMap;
use tugboat_resources::manifests::apps::v1::{
    Deployment, DeploymentStatus, ReplicaSet, ReplicaSetSpec,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta};

pub(super) const REPLICASET_UPDATE_STRATEGY_ANNOTATION: &str = "tugboat.cloud/update-strategy";
pub(super) const REPLICASET_UPDATE_STRATEGY_ALL: &str = "all";

pub(super) fn owner_reference_for_deployment(dep: &Deployment) -> OwnerReference {
    owner_reference_for(dep)
}

pub(super) fn is_owned_by_deployment(rs: &ReplicaSet, dep: &Deployment) -> bool {
    is_controlled_by(rs.object_meta().as_ref(), dep, "Deployment")
}

pub(super) fn managed_replicasets<'a>(
    replicasets: &'a [ReplicaSet],
    dep: &Deployment,
) -> Vec<&'a ReplicaSet> {
    replicasets
        .iter()
        .filter(|rs| is_owned_by_deployment(rs, dep))
        .collect()
}

pub(super) fn replicaset_creation_sort_key(rs: &ReplicaSet) -> (i64, i32, String) {
    let (seconds, nanos) = rs
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.creation_timestamp.as_ref())
        .map(|time| (time.seconds, time.nanos))
        .unwrap_or((0, 0));
    (seconds, nanos, rs.name().unwrap_or_default().to_string())
}

pub(super) fn active_replicaset<'a>(replicasets: &'a [&ReplicaSet]) -> Option<&'a ReplicaSet> {
    replicasets
        .iter()
        .copied()
        .max_by_key(|rs| replicaset_creation_sort_key(rs))
}

pub(super) fn old_replicasets<'a>(
    managed: &'a [&ReplicaSet],
    active_rs_name: Option<&str>,
) -> Vec<&'a ReplicaSet> {
    managed
        .iter()
        .copied()
        .filter(|rs| rs.name() != active_rs_name)
        .collect()
}

pub(super) fn has_old_running_replicas(old_replicasets: &[&ReplicaSet]) -> bool {
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

pub(super) fn stale_replicasets_for_cleanup<'a>(
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

pub(super) fn revision_history_limit(dep: &Deployment) -> usize {
    dep.spec
        .as_ref()
        .and_then(|spec| spec.revision_history_limit)
        .unwrap_or(0)
        .max(0) as usize
}

pub(super) fn build_replicaset_with_replicas(
    dep: &Deployment,
    hash: &str,
    replicas: Option<i32>,
) -> ReplicaSet {
    let dep_name = dep.name().unwrap_or_default();
    let dep_namespace = dep.namespace().map(str::to_string);
    let dep_spec = dep.spec.as_ref().cloned().unwrap_or_default();
    let mut selector = dep_spec.selector;
    selector.insert(SHIP_TEMPLATE_HASH_LABEL.to_string(), hash.to_string());

    let mut labels: HashMap<String, String> = selector.clone();
    labels.insert(SHIP_TEMPLATE_HASH_LABEL.to_string(), hash.to_string());

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

pub(super) fn build_replicaset(dep: &Deployment, hash: &str) -> ReplicaSet {
    let replicas = dep.spec.as_ref().and_then(|spec| spec.replicas);
    build_replicaset_with_replicas(dep, hash, replicas)
}

pub(super) fn update_replicaset_for_in_place(
    active_rs: &ReplicaSet,
    dep: &Deployment,
) -> ReplicaSet {
    let mut updated_rs = active_rs.clone();
    let dep_spec = dep.spec.as_ref().cloned().unwrap_or_default();
    let rs_spec = updated_rs.spec.get_or_insert_with(ReplicaSetSpec::default);
    rs_spec.ship_template = dep_spec.ship_template;
    rs_spec.replicas = dep_spec.replicas;
    updated_rs
}

pub(super) fn update_replicaset_replicas(
    active_rs: &ReplicaSet,
    replicas: Option<i32>,
) -> Option<ReplicaSet> {
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

pub(super) fn set_in_place_update_strategy(rs: &mut ReplicaSet, strategy: Option<&str>) {
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

pub(super) fn replicaset_spec_replicas(rs: &ReplicaSet) -> i32 {
    rs.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(0)
}

pub(super) fn replicaset_status_replicas(rs: &ReplicaSet) -> i32 {
    rs.status
        .as_ref()
        .map(|status| status.replicas)
        .unwrap_or_else(|| replicaset_spec_replicas(rs))
}

pub(super) fn replicaset_ready_replicas(rs: &ReplicaSet) -> i32 {
    rs.status
        .as_ref()
        .map(|status| status.ready_replicas)
        .unwrap_or(0)
}

pub(super) fn rolling_update_limits(dep: &Deployment) -> (i32, i32) {
    let strategy = dep.spec.as_ref().and_then(|spec| spec.strategy.as_ref());
    let rolling = strategy.and_then(|strategy| strategy.rolling_update.as_ref());
    let max_surge = rolling.and_then(|rolling| rolling.max_surge).unwrap_or(1);
    let max_unavailable = rolling
        .and_then(|rolling| rolling.max_unavailable)
        .unwrap_or(1);
    (max_surge.max(0), max_unavailable.max(0))
}

pub(super) fn sum_status_replicas(replicasets: &[&ReplicaSet]) -> i32 {
    replicasets
        .iter()
        .map(|rs| replicaset_status_replicas(rs))
        .sum()
}

pub(super) fn sum_ready_replicas(replicasets: &[&ReplicaSet]) -> i32 {
    replicasets
        .iter()
        .map(|rs| replicaset_ready_replicas(rs))
        .sum()
}

pub(super) fn build_deployment_status(
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

pub(super) fn next_rolling_update_rotation_targets(
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
