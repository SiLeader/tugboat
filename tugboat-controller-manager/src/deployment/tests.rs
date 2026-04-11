use super::rs_ops::{
    REPLICASET_UPDATE_STRATEGY_ALL, REPLICASET_UPDATE_STRATEGY_ANNOTATION, active_replicaset,
    build_deployment_status, build_replicaset, build_replicaset_with_replicas,
    has_old_running_replicas, is_owned_by_deployment, managed_replicasets,
    next_rolling_update_rotation_targets, old_replicasets, owner_reference_for_deployment,
    replicaset_ready_replicas, revision_history_limit, rolling_update_limits,
    set_in_place_update_strategy, stale_replicasets_for_cleanup, update_replicaset_for_in_place,
    update_replicaset_replicas,
};
use super::template_hash::{deployment_template, replicaset_has_template_hash, template_hash};
use crate::error::ControllerError;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::apps::v1::{
    Deployment, DeploymentSpec, DeploymentStatus, DeploymentStrategy, ReplicaSet, ReplicaSetSpec,
    ReplicaSetStatus, RollingUpdateStrategy, ShipTemplateSpec,
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
                    image: "example.com/images/demo:v1".to_string(),
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
                    image: "example.com/images/demo:v1".to_string(),
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

    let rs = build_replicaset_with_replicas(&dep, &hash, Some(0));

    assert_eq!(rs.spec.as_ref().unwrap().replicas, Some(0));
}

#[test]
fn deployment_template_returns_error_when_missing() {
    let mut dep = deployment();
    dep.spec.as_mut().unwrap().ship_template = None;

    let result = deployment_template(&dep);

    assert!(matches!(
        result,
        Err(ControllerError::MissingDeploymentTemplate { .. })
    ));
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
    assert_eq!(template_hash(template).len(), 16);
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
            image: "example.com/images/demo:v1".to_string(),
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
            image: "example.com/images/demo:v1".to_string(),
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

    let replicasets = [&zeroed, &still_running];
    let stale = stale_replicasets_for_cleanup(&replicasets, 0);

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

    let replicasets = [&newer, &oldest];
    let stale = stale_replicasets_for_cleanup(&replicasets, 1);

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].name(), Some("demo-rs-1"));
}

#[test]
fn deployment_revision_history_limit_defaults_to_zero() {
    let dep = deployment();

    assert_eq!(revision_history_limit(&dep), 0);
}

#[test]
fn deployment_revision_history_limit_uses_spec_value() {
    let mut dep = deployment();
    dep.spec.as_mut().unwrap().revision_history_limit = Some(2);

    assert_eq!(revision_history_limit(&dep), 2);
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
