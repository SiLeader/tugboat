mod scale;
mod ship_builder;
mod status;
mod template_update;

use crate::base::TugboatController;
use crate::change_classifier::{TemplateChangeKind, classify_template_change};
use crate::error::ControllerError;
use scale::{
    delete_ship_ignore_not_found, is_owned_by_replicaset, owned_ships, reconcile_ship_count,
};
use status::build_replicaset_status;
use template_update::{
    migration_blocks_template_update, reconcile_template_updates,
    should_wait_for_ready_before_update,
};
use tracing::debug;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::apps::v1::{ReplicaSet, ShipTemplateSpec};
use tugboat_resources::manifests::core::v1::{RuntimeClass, Ship};

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

async fn resolve_runtime_class(
    client: &TugboatClient,
    template: Option<&ShipTemplateSpec>,
) -> Result<Option<RuntimeClass>, ControllerError> {
    let runtime_class_name = template
        .and_then(|template| template.spec.as_ref())
        .and_then(|spec| spec.runtime_class.as_deref())
        .map(str::trim)
        .filter(|name| !name.is_empty());

    let Some(runtime_class_name) = runtime_class_name else {
        return Ok(None);
    };

    let runtime_class_api: Api<RuntimeClass> = Api::all(client.clone());
    runtime_class_api
        .get(runtime_class_name)
        .await
        .map_err(Into::into)
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
        debug!(
            rs = rs.name().unwrap_or_default(),
            desired,
            matching = matching_ships.len(),
            "reconciling replicaset"
        );
        if let Some(action) =
            reconcile_ship_count(&ship_api, &rs, rs_spec, &matching_ships, desired).await?
        {
            return Ok(action);
        }

        let template_spec = rs_spec
            .ship_template
            .as_ref()
            .and_then(|template| template.spec.as_ref())
            .cloned()
            .unwrap_or_default();
        let runtime_class =
            resolve_runtime_class(&self.client, rs_spec.ship_template.as_ref()).await?;
        let desired_template = rs_spec.ship_template.clone().unwrap_or_default();
        let change_kind = matching_ships
            .iter()
            .find_map(|ship| ship.spec.as_ref())
            .map(|current_spec| {
                classify_template_change(
                    &ShipTemplateSpec {
                        spec: Some(current_spec.clone()),
                        ..Default::default()
                    },
                    &desired_template,
                    runtime_class.as_ref(),
                )
            })
            .unwrap_or(TemplateChangeKind::NoChange);

        let migration_blocks_update =
            migration_blocks_template_update(&matching_ships, &template_spec, change_kind);

        if should_wait_for_ready_before_update(&matching_ships, &template_spec, change_kind) {
            return Ok(Action::requeue(std::time::Duration::from_secs(5)));
        }

        if let Some(action) = reconcile_template_updates(
            &ship_api,
            &rs,
            &matching_ships,
            &template_spec,
            change_kind,
            migration_blocks_update,
        )
        .await?
        {
            return Ok(action);
        }

        let new_status = build_replicaset_status(&matching_ships);

        if rs.status.as_ref() != Some(&new_status) {
            let mut updated = rs.clone();
            updated.status = Some(new_status);
            let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
            rs_api
                .replace_status(rs.name().unwrap_or_default(), updated)
                .await?;
        }

        // Requeue while not all replicas are ready so ship status changes are picked up.
        if new_status.ready_replicas < desired as i32 {
            return Ok(Action::requeue(std::time::Duration::from_secs(5)));
        }

        Ok(Action::await_change())
    }

    async fn reconcile_deleted(&self, rs: ReplicaSet) -> Result<Action, ControllerError> {
        let Some(namespace) = rs.namespace() else {
            return Err(ControllerError::MissingNamespace("ReplicaSet"));
        };
        let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
        let ships = ship_api.list().await?;
        for ship in ships
            .iter()
            .filter(|ship| is_owned_by_replicaset(ship, &rs))
        {
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
        scale::{
            excess_ships_to_delete, is_owned_by_replicaset as is_owned_by, owned_ships,
            ship_creation_sort_key,
        },
        ship_builder::{build_ship, generate_suffix, owner_reference_for_replicaset},
        status::{build_replicaset_status, ship_is_ready},
        template_update::{
            UPDATE_STRATEGY_ALL, UPDATE_STRATEGY_ANNOTATION, apply_template_spec,
            needs_spec_update, update_strategy,
        },
    };
    use crate::workload::matches_selector;
    use std::collections::HashMap;
    use tugboat_resources::manifests::apps::v1::{
        ReplicaSet, ReplicaSetSpec, ReplicaSetStatus, ShipTemplateSpec,
    };
    use tugboat_resources::manifests::core::v1::{
        ProjectedVolumeSource, ServiceAccountTokenProjection, Ship, ShipCondition,
        ShipMigrationStatus, ShipSpec, ShipStatus, ShipVolume, VolumeProjection,
    };
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
    use tugboat_resources::{ObjectMetaResource, Resource};

    fn base_ship_spec() -> ShipSpec {
        ShipSpec {
            image: "example.com/images/demo:latest".to_string(),
            ship_class: "standard".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn generated_suffix_has_expected_shape() {
        let suffix = generate_suffix();

        assert_eq!(suffix.len(), 5);
        assert!(
            suffix
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        );
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
                    image: "example.com/images/demo:latest".to_string(),
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
            Some("example.com/images/demo:latest")
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
    fn excess_deletion_prefers_non_migrating_ships() {
        let stable = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("stable".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 10,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let migrating = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("migrating".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 20,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: "Migrating".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let owned = vec![&stable, &migrating];

        let to_delete = excess_ships_to_delete(&owned, 1);

        assert_eq!(to_delete.len(), 1);
        assert_eq!(to_delete[0].name(), Some("stable"));
    }

    #[test]
    fn excess_deletion_returns_none_when_only_migrating_ships_are_excess() {
        let stable = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("stable".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 10,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: "Ready".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let migrating = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("migrating".to_string()),
                creation_timestamp: Some(tugboat_resources::manifests::meta::v1::Time {
                    seconds: 20,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: "Migrating".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let owned = vec![&stable, &migrating];

        let to_delete = excess_ships_to_delete(&owned, 0);

        assert!(to_delete.is_empty());
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
    fn admission_defaulted_service_account_projection_does_not_require_update() {
        let template_spec = base_ship_spec();
        let mut ship_spec = template_spec.clone();
        ship_spec.service_account_name = Some("default".to_string());
        ship_spec.volumes.push(ShipVolume {
            name: "serviceaccount-token".to_string(),
            projected: Some(ProjectedVolumeSource {
                sources: vec![VolumeProjection {
                    service_account_token: Some(ServiceAccountTokenProjection {
                        expiration_seconds: Some(3600),
                        path: "token".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                default_mode: Some(0o644),
            }),
            ..Default::default()
        });

        assert!(!needs_spec_update(&template_spec, &ship_spec));
    }

    #[test]
    fn runtime_significant_fields_require_update() {
        let template_spec = base_ship_spec();
        let mut ship_spec = template_spec.clone();
        ship_spec.image = "example.com/images/demo:v2".to_string();

        assert!(needs_spec_update(&template_spec, &ship_spec));
    }

    #[test]
    fn runtime_class_change_requires_update() {
        let template_spec = base_ship_spec();
        let mut ship_spec = template_spec.clone();
        ship_spec.runtime_class = Some("kata".to_string());

        assert!(needs_spec_update(&template_spec, &ship_spec));
    }

    #[test]
    fn applying_template_spec_preserves_scheduling_fields() {
        let template_spec = ShipSpec {
            image: "example.com/images/demo:v2".to_string(),
            ship_class: "large".to_string(),
            runtime_class: Some("kata".to_string()),
            ..Default::default()
        };
        let mut ship = Ship {
            spec: Some(ShipSpec {
                image: "example.com/images/demo:latest".to_string(),
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

        assert_eq!(updated.image, "example.com/images/demo:v2");
        assert_eq!(updated.ship_class, "large");
        assert_eq!(updated.runtime_class.as_deref(), Some("kata"));
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
