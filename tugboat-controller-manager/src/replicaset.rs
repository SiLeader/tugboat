use crate::base::TugboatController;
use crate::error::ControllerError;
use rand::distr::{Alphanumeric, SampleString};
use rand::rng;
use std::collections::HashMap;
use std::time::Duration;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::apps::v1::{ReplicaSet, ReplicaSetStatus, ShipTemplateSpec};
use tugboat_resources::manifests::core::v1::{Ship, ShipSpec};
use tugboat_resources::manifests::meta::v1::OwnerReference;
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta};

const UPDATE_STRATEGY_ANNOTATION: &str = "tugboat.dev/update-strategy";
const UPDATE_STRATEGY_ALL: &str = "all";

#[derive(Clone)]
struct ReplicaSetReconciler {
    client: TugboatClient,
}

pub(crate) struct ReplicaSetController {
    controller: Controller<ReplicaSet>,
    reconciler: ReplicaSetReconciler,
}

impl ReplicaSetController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: ReplicaSetReconciler { client },
        }
    }
}

fn matches_selector(labels: &HashMap<String, String>, selector: &HashMap<String, String>) -> bool {
    selector
        .iter()
        .all(|(key, value)| labels.get(key).is_some_and(|label| label == value))
}

fn owner_reference_for_replicaset(rs: &ReplicaSet) -> OwnerReference {
    OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "ReplicaSet".to_string(),
        name: rs.name().unwrap_or_default().to_string(),
        uid: rs
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.as_deref())
            .unwrap_or_default()
            .to_string(),
        controller: Some(true),
    }
}

fn ship_matches_selector(ship: &Ship, selector: &HashMap<String, String>) -> bool {
    ship.object_meta
        .as_ref()
        .is_some_and(|meta| matches_selector(&meta.labels, selector))
}

fn owned_ships<'a>(ships: &'a [Ship], selector: &HashMap<String, String>) -> Vec<&'a Ship> {
    ships
        .iter()
        .filter(|ship| ship_matches_selector(ship, selector))
        .collect()
}

fn is_owned_by(ship: &Ship, rs: &ReplicaSet) -> bool {
    let Some(rs_uid) = rs
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.uid.as_deref())
    else {
        return false;
    };
    ship.object_meta.as_ref().is_some_and(|meta| {
        meta.owner_references
            .iter()
            .any(|owner_ref| owner_ref.uid == rs_uid)
    })
}

fn ship_creation_sort_key(ship: &Ship) -> (i64, i32, String) {
    let (seconds, nanos) = ship
        .object_meta
        .as_ref()
        .and_then(|meta| meta.creation_timestamp.as_ref())
        .map(|time| (time.seconds, time.nanos))
        .unwrap_or((0, 0));
    (seconds, nanos, ship.name().unwrap_or_default().to_string())
}

fn excess_ships_to_delete<'a>(owned_ships: &[&'a Ship], desired: usize) -> Vec<&'a Ship> {
    if owned_ships.len() <= desired {
        return Vec::new();
    }

    let mut sorted = owned_ships.to_vec();
    sorted.sort_by_key(|ship| std::cmp::Reverse(ship_creation_sort_key(ship)));
    sorted.truncate(owned_ships.len() - desired);
    sorted
}

fn generate_suffix() -> String {
    Alphanumeric.sample_string(&mut rng(), 5)
}

fn needs_spec_update(template_spec: &ShipSpec, ship_spec: &ShipSpec) -> bool {
    template_spec.image != ship_spec.image
        || template_spec.ship_class != ship_spec.ship_class
        || template_spec.network_class_ref != ship_spec.network_class_ref
        || template_spec.uefi != ship_spec.uefi
        || template_spec.volume_claim_ref != ship_spec.volume_claim_ref
        || template_spec.volumes != ship_spec.volumes
}

fn ship_is_ready(ship: &Ship) -> bool {
    ship.status.as_ref().is_some_and(|status| {
        status
            .conditions
            .iter()
            .any(|condition| condition.status == "Running")
    })
}

fn build_replicaset_status(owned_ships: &[&Ship]) -> ReplicaSetStatus {
    ReplicaSetStatus {
        replicas: owned_ships.len() as i32,
        ready_replicas: owned_ships
            .iter()
            .filter(|ship| ship_is_ready(ship))
            .count() as i32,
    }
}

fn apply_template_spec(ship: &mut Ship, template_spec: &ShipSpec) {
    let ship_spec = ship.spec.get_or_insert_with(ShipSpec::default);
    ship_spec.image = template_spec.image.clone();
    ship_spec.ship_class = template_spec.ship_class.clone();
    ship_spec.network_class_ref = template_spec.network_class_ref.clone();
    ship_spec.uefi = template_spec.uefi;
    ship_spec.volume_claim_ref = template_spec.volume_claim_ref.clone();
    ship_spec.volumes = template_spec.volumes.clone();
}

fn update_strategy(rs: &ReplicaSet) -> Option<&str> {
    rs.object_meta()
        .as_ref()
        .and_then(|meta| meta.annotations.get(UPDATE_STRATEGY_ANNOTATION))
        .map(String::as_str)
}

async fn delete_ship_ignore_not_found(
    ship_api: &Api<Ship>,
    name: &str,
) -> Result<(), ControllerError> {
    match ship_api.delete(name).await {
        Ok(_) => Ok(()),
        Err(tugboat_client::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn build_ship(rs: &ReplicaSet, template: &ShipTemplateSpec) -> Ship {
    let mut metadata = template.metadata.clone().unwrap_or_default();
    metadata.name = Some(format!(
        "{}-{}",
        rs.name().unwrap_or_default(),
        generate_suffix()
    ));
    metadata.namespace = rs.namespace().map(str::to_string);
    metadata.labels.extend(
        rs.spec
            .as_ref()
            .map(|spec| spec.selector.clone())
            .unwrap_or_default(),
    );
    metadata
        .owner_references
        .push(owner_reference_for_replicaset(rs));

    let mut ship = Ship {
        object_meta: Some(metadata),
        spec: Some(template.spec.clone().unwrap_or_default()),
        status: None,
        ..Default::default()
    };
    ship.set_type_meta(Ship::type_meta());
    ship
}

#[async_trait::async_trait]
impl TugboatController for ReplicaSetController {
    fn name(&self) -> &str {
        "replicaset"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

impl ReplicaSetReconciler {
    async fn reconcile_applied(&self, rs: ReplicaSet) -> Result<Action, ControllerError> {
        let Some(namespace) = rs.namespace() else {
            return Err(ControllerError::MissingNamespace("ReplicaSet"));
        };
        let spec = rs
            .spec
            .as_ref()
            .map(|spec| &spec.selector)
            .zip(rs.spec.as_ref())
            .ok_or(ControllerError::MissingReplicaSetSpec {
                namespace: namespace.to_string(),
                name: rs.name().unwrap_or_default().to_string(),
            })?;
        let (selector, rs_spec) = spec;
        let desired = rs_spec.replicas.unwrap_or(1).max(0) as usize;
        let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let ships = ship_api.list().await?;
        let mut matching_ships = owned_ships(&ships, selector);
        matching_ships.sort_by_key(|ship| ship.name().unwrap_or_default());

        for _ in matching_ships.len()..desired {
            let ship = build_ship(&rs, &rs_spec.ship_template.clone().unwrap_or_default());
            match ship_api.create(ship).await {
                Ok(_) => {}
                Err(tugboat_client::Error::Api(status)) if status.code == 409 => {}
                Err(err) => return Err(err.into()),
            }
        }

        if matching_ships.len() > desired {
            for ship in excess_ships_to_delete(&matching_ships, desired) {
                if let Some(name) = ship.name() {
                    delete_ship_ignore_not_found(&ship_api, name).await?;
                }
            }
            return Ok(Action::requeue(Duration::from_secs(5)));
        }

        let template_spec = rs_spec
            .ship_template
            .as_ref()
            .and_then(|template| template.spec.as_ref())
            .cloned()
            .unwrap_or_default();

        if matching_ships.iter().any(|ship| {
            ship.spec
                .as_ref()
                .is_some_and(|ship_spec| needs_spec_update(&template_spec, ship_spec))
                && !ship_is_ready(ship)
        }) {
            return Ok(Action::requeue(Duration::from_secs(5)));
        }

        if update_strategy(&rs) == Some(UPDATE_STRATEGY_ALL) {
            let mut updated_any = false;
            for ship in &matching_ships {
                let Some(ship_spec) = ship.spec.as_ref() else {
                    continue;
                };
                if !needs_spec_update(&template_spec, ship_spec) {
                    continue;
                }
                let Some(name) = ship.name() else {
                    continue;
                };
                let mut updated = (*ship).clone();
                apply_template_spec(&mut updated, &template_spec);
                ship_api.replace(name, updated).await?;
                updated_any = true;
            }
            if updated_any {
                return Ok(Action::requeue(Duration::from_secs(5)));
            }
        }

        for ship in matching_ships {
            let Some(ship_spec) = ship.spec.as_ref() else {
                continue;
            };
            if !needs_spec_update(&template_spec, ship_spec) {
                continue;
            }
            let Some(name) = ship.name() else {
                continue;
            };
            let mut updated = ship.clone();
            apply_template_spec(&mut updated, &template_spec);
            ship_api.replace(name, updated).await?;
            return Ok(Action::requeue(Duration::from_secs(5)));
        }

        let latest_ships = ship_api.list().await?;
        let latest_owned_ships = owned_ships(&latest_ships, selector);
        let new_status = build_replicaset_status(&latest_owned_ships);

        if rs.status.as_ref() != Some(&new_status) {
            let mut updated = rs.clone();
            updated.status = Some(new_status);
            let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
            rs_api
                .replace(rs.name().unwrap_or_default(), updated)
                .await?;
        }

        Ok(Action::await_change())
    }

    async fn reconcile_deleted(&self, rs: ReplicaSet) -> Result<Action, ControllerError> {
        let Some(namespace) = rs.namespace() else {
            return Err(ControllerError::MissingNamespace("ReplicaSet"));
        };
        let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let ships = ship_api.list().await?;
        for ship in ships.iter().filter(|ship| is_owned_by(ship, &rs)) {
            if let Some(name) = ship.name() {
                delete_ship_ignore_not_found(&ship_api, name).await?;
            }
        }
        Ok(Action::await_change())
    }
}

#[async_trait::async_trait]
impl Reconciler<ReplicaSet> for ReplicaSetReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<ReplicaSet>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(rs) => self.reconcile_applied(rs).await,
            ReconcileEvent::Deleted(rs) => self.reconcile_deleted(rs).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        UPDATE_STRATEGY_ALL, UPDATE_STRATEGY_ANNOTATION, apply_template_spec,
        build_replicaset_status, build_ship, excess_ships_to_delete, generate_suffix, is_owned_by,
        matches_selector, needs_spec_update, owned_ships, owner_reference_for_replicaset,
        ship_creation_sort_key, ship_is_ready, update_strategy,
    };
    use std::collections::HashMap;
    use tugboat_resources::manifests::apps::v1::{
        ReplicaSet, ReplicaSetSpec, ReplicaSetStatus, ShipTemplateSpec,
    };
    use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipSpec, ShipStatus};
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
    use tugboat_resources::{ObjectMetaResource, Resource};

    fn base_ship_spec() -> ShipSpec {
        ShipSpec {
            image: "ghcr.io/example/demo:latest".to_string(),
            ship_class: "standard".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn generated_suffix_has_expected_shape() {
        let suffix = generate_suffix();

        assert_eq!(suffix.len(), 5);
        assert!(suffix.chars().all(|ch| ch.is_ascii_alphanumeric()));
    }

    #[test]
    fn build_ship_merges_selector_labels_and_owner_reference() {
        let rs = ReplicaSet {
            object_meta: Some(ObjectMeta {
                name: Some("demo-rs".to_string()),
                namespace: Some("default".to_string()),
                uid: Some("rs-uid".to_string()),
                ..Default::default()
            }),
            spec: Some(ReplicaSetSpec {
                selector: HashMap::from([("app".to_string(), "demo".to_string())]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ship = build_ship(
            &rs,
            &ShipTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: HashMap::from([("tier".to_string(), "backend".to_string())]),
                    ..Default::default()
                }),
                spec: Some(ShipSpec {
                    image: "ghcr.io/example/demo:latest".to_string(),
                    ship_class: "standard".to_string(),
                    ..Default::default()
                }),
            },
        );
        let meta = ship.object_meta.as_ref().unwrap();

        assert_eq!(ship.type_meta, Some(Ship::type_meta()));
        assert_eq!(meta.namespace.as_deref(), Some("default"));
        assert!(
            meta.name
                .as_deref()
                .is_some_and(|name| name.starts_with("demo-rs-"))
        );
        assert_eq!(meta.labels.get("app"), Some(&"demo".to_string()));
        assert_eq!(meta.labels.get("tier"), Some(&"backend".to_string()));
        assert_eq!(
            meta.owner_references,
            vec![owner_reference_for_replicaset(&rs)]
        );
        assert_eq!(
            ship.spec.as_ref().map(|spec| spec.image.as_str()),
            Some("ghcr.io/example/demo:latest")
        );
    }

    #[test]
    fn selector_matches_when_all_entries_exist() {
        let labels = HashMap::from([
            ("app".to_string(), "demo".to_string()),
            ("tier".to_string(), "backend".to_string()),
        ]);
        let selector = HashMap::from([
            ("app".to_string(), "demo".to_string()),
            ("tier".to_string(), "backend".to_string()),
        ]);

        assert!(matches_selector(&labels, &selector));
    }

    #[test]
    fn selector_rejects_missing_or_different_labels() {
        let labels = HashMap::from([("app".to_string(), "demo".to_string())]);
        let selector = HashMap::from([
            ("app".to_string(), "demo".to_string()),
            ("tier".to_string(), "backend".to_string()),
        ]);

        assert!(!matches_selector(&labels, &selector));
    }

    #[test]
    fn filters_owned_ships_by_selector() {
        let selector = HashMap::from([("app".to_string(), "demo".to_string())]);
        let matching = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("matching".to_string()),
                labels: HashMap::from([("app".to_string(), "demo".to_string())]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let other = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("other".to_string()),
                labels: HashMap::from([("app".to_string(), "other".to_string())]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ships = [matching, other];
        let owned = owned_ships(&ships, &selector);

        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].name(), Some("matching"));
    }

    #[test]
    fn detects_ships_owned_by_replicaset_uid() {
        let rs = ReplicaSet {
            object_meta: Some(ObjectMeta {
                uid: Some("rs-uid".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let owned = Ship {
            object_meta: Some(ObjectMeta {
                owner_references: vec![OwnerReference {
                    uid: "rs-uid".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let other = Ship {
            object_meta: Some(ObjectMeta {
                owner_references: vec![OwnerReference {
                    uid: "other".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(is_owned_by(&owned, &rs));
        assert!(!is_owned_by(&other, &rs));
    }

    #[test]
    fn sorts_ships_by_creation_timestamp_then_name() {
        let newer = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("b".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 20,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let older = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("a".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 10,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(ship_creation_sort_key(&newer) > ship_creation_sort_key(&older));
    }

    #[test]
    fn picks_newest_ships_for_excess_deletion() {
        let ship_a = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("a".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 10,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ship_b = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("b".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 20,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ship_c = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("c".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 30,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let owned = vec![&ship_a, &ship_b, &ship_c];

        let to_delete = excess_ships_to_delete(&owned, 1);

        assert_eq!(to_delete.len(), 2);
        assert_eq!(to_delete[0].name(), Some("c"));
        assert_eq!(to_delete[1].name(), Some("b"));
    }

    #[test]
    fn builds_replicaset_owner_reference() {
        let rs = ReplicaSet {
            object_meta: Some(ObjectMeta {
                name: Some("demo-rs".to_string()),
                uid: Some("rs-uid".to_string()),
                ..Default::default()
            }),
            spec: Some(ReplicaSetSpec::default()),
            ..Default::default()
        };

        assert_eq!(
            owner_reference_for_replicaset(&rs),
            OwnerReference {
                api_version: "apps/v1".to_string(),
                kind: "ReplicaSet".to_string(),
                name: "demo-rs".to_string(),
                uid: "rs-uid".to_string(),
                controller: Some(true),
            }
        );
    }

    #[test]
    fn scheduling_only_fields_do_not_require_update() {
        let template_spec = base_ship_spec();
        let mut ship_spec = template_spec.clone();
        ship_spec.node_name = Some("node-a".to_string());
        ship_spec.scheduler_name = Some("scheduler".to_string());

        assert!(!needs_spec_update(&template_spec, &ship_spec));
    }

    #[test]
    fn target_node_name_does_not_require_update() {
        let template_spec = base_ship_spec();
        let mut ship_spec = template_spec.clone();
        ship_spec.target_node_name = Some("node-b".to_string());

        assert!(!needs_spec_update(&template_spec, &ship_spec));
    }

    #[test]
    fn runtime_significant_fields_require_update() {
        let template_spec = base_ship_spec();
        let mut ship_spec = template_spec.clone();
        ship_spec.image = "ghcr.io/example/demo:v2".to_string();

        assert!(needs_spec_update(&template_spec, &ship_spec));
    }

    #[test]
    fn applying_template_spec_preserves_scheduling_fields() {
        let template_spec = ShipSpec {
            image: "ghcr.io/example/demo:v2".to_string(),
            ship_class: "large".to_string(),
            ..Default::default()
        };
        let mut ship = Ship {
            spec: Some(ShipSpec {
                image: "ghcr.io/example/demo:latest".to_string(),
                ship_class: "small".to_string(),
                node_name: Some("node-a".to_string()),
                scheduler_name: Some("scheduler".to_string()),
                target_node_name: Some("node-b".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        apply_template_spec(&mut ship, &template_spec);
        let updated = ship.spec.as_ref().unwrap();

        assert_eq!(updated.image, "ghcr.io/example/demo:v2");
        assert_eq!(updated.ship_class, "large");
        assert_eq!(updated.node_name.as_deref(), Some("node-a"));
        assert_eq!(updated.scheduler_name.as_deref(), Some("scheduler"));
        assert_eq!(updated.target_node_name.as_deref(), Some("node-b"));
    }

    #[test]
    fn ship_is_ready_when_running_condition_exists() {
        let ship = Ship {
            status: Some(ShipStatus {
                conditions: vec![ShipCondition {
                    status: "Running".to_string(),
                    message: "ok".to_string(),
                    timestamp: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(ship_is_ready(&ship));
    }

    #[test]
    fn ship_is_not_ready_without_running_condition() {
        let ship = Ship {
            status: Some(ShipStatus {
                conditions: vec![ShipCondition {
                    status: "VmCreating".to_string(),
                    message: "creating".to_string(),
                    timestamp: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(!ship_is_ready(&ship));
    }

    #[test]
    fn builds_replicaset_status_from_owned_ship_counts() {
        let ready_ship = Ship {
            status: Some(ShipStatus {
                conditions: vec![ShipCondition {
                    status: "Running".to_string(),
                    message: "ok".to_string(),
                    timestamp: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let pending_ship = Ship {
            status: Some(ShipStatus {
                conditions: vec![ShipCondition {
                    status: "VmCreating".to_string(),
                    message: "creating".to_string(),
                    timestamp: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let ships = [&ready_ship, &pending_ship];

        assert_eq!(
            build_replicaset_status(&ships),
            ReplicaSetStatus {
                replicas: 2,
                ready_replicas: 1,
            }
        );
    }

    #[test]
    fn update_strategy_reads_annotation() {
        let rs = ReplicaSet {
            object_meta: Some(ObjectMeta {
                annotations: HashMap::from([(
                    UPDATE_STRATEGY_ANNOTATION.to_string(),
                    UPDATE_STRATEGY_ALL.to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(update_strategy(&rs), Some(UPDATE_STRATEGY_ALL));
    }
}
