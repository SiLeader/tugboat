use crate::base::TugboatController;
use crate::error::ControllerError;
use std::collections::HashMap;
use std::time::Duration;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::apps::v1::{
    Fleet, FleetComponent, FleetStatus, ReplicaSet, ReplicaSetSpec, ShipTemplateSpec,
};
use tugboat_resources::manifests::core::v1::{ClusterNetworkClass, ShipNetworkClassReference};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta};

const FLEET_NAME_LABEL: &str = "fleet-name";
const FLEET_COMPONENT_LABEL: &str = "fleet-component";

#[derive(Clone)]
struct FleetReconciler {
    client: TugboatClient,
}

pub(crate) struct FleetController {
    controller: Controller<Fleet>,
    reconciler: FleetReconciler,
}

impl FleetController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: FleetReconciler { client },
        }
    }
}

fn owner_reference_for_fleet(fleet: &Fleet) -> OwnerReference {
    OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "Fleet".to_string(),
        name: fleet.name().unwrap_or_default().to_string(),
        uid: fleet
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.as_deref())
            .unwrap_or_default()
            .to_string(),
        controller: Some(true),
    }
}

fn is_owned_by_fleet(rs: &ReplicaSet, fleet: &Fleet) -> bool {
    let fleet_name = fleet.name().unwrap_or_default();
    let fleet_uid = fleet
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.uid.as_deref());

    rs.object_meta().as_ref().is_some_and(|meta| {
        meta.owner_references.iter().any(|owner_ref| {
            owner_ref.kind == "Fleet"
                && owner_ref.name == fleet_name
                && fleet_uid
                    .map(|uid| owner_ref.uid == uid)
                    .unwrap_or_else(|| owner_ref.uid.is_empty())
        })
    })
}

fn managed_replicasets<'a>(replicasets: &'a [ReplicaSet], fleet: &Fleet) -> Vec<&'a ReplicaSet> {
    replicasets
        .iter()
        .filter(|rs| is_owned_by_fleet(rs, fleet))
        .collect()
}

fn component_name(rs: &ReplicaSet) -> Option<&str> {
    rs.object_meta()
        .as_ref()
        .and_then(|meta| meta.labels.get(FLEET_COMPONENT_LABEL))
        .map(String::as_str)
        .or_else(|| {
            rs.spec
                .as_ref()
                .and_then(|spec| spec.selector.get(FLEET_COMPONENT_LABEL))
                .map(String::as_str)
        })
}

fn find_rs_for_component<'a>(
    replicasets: &'a [&ReplicaSet],
    target_component_name: &str,
) -> Option<&'a ReplicaSet> {
    replicasets
        .iter()
        .copied()
        .find(|rs| component_name(rs) == Some(target_component_name))
}

fn inject_fleet_network(template: &mut ShipTemplateSpec, network_class_name: &str) {
    let spec = template.spec.get_or_insert_with(Default::default);
    if !spec
        .network_class_ref
        .iter()
        .any(|network| network.name == network_class_name)
    {
        spec.network_class_ref.push(ShipNetworkClassReference {
            kind: "ClusterNetworkClass".to_string(),
            name: network_class_name.to_string(),
            api_group: "core".to_string(),
        });
    }
}

fn build_replicaset_for_component(
    fleet: &Fleet,
    component: &FleetComponent,
    network_class_name: &str,
) -> ReplicaSet {
    let fleet_name = fleet.name().unwrap_or_default();
    let namespace = fleet.namespace().map(str::to_string);
    let selector: HashMap<String, String> = [
        (FLEET_NAME_LABEL.to_string(), fleet_name.to_string()),
        (FLEET_COMPONENT_LABEL.to_string(), component.name.clone()),
    ]
    .into_iter()
    .collect();

    let mut template = component.ship_template.clone().unwrap_or_default();
    inject_fleet_network(&mut template, network_class_name);

    let mut rs = ReplicaSet {
        object_meta: Some(ObjectMeta {
            name: Some(format!("{fleet_name}-{}", component.name)),
            namespace,
            labels: selector.clone(),
            owner_references: vec![owner_reference_for_fleet(fleet)],
            ..Default::default()
        }),
        spec: Some(ReplicaSetSpec {
            replicas: Some(component.replicas),
            selector,
            ship_template: Some(template),
        }),
        status: None,
        ..Default::default()
    };
    rs.set_type_meta(ReplicaSet::type_meta());
    rs
}

fn update_replicaset_for_component(
    rs: &ReplicaSet,
    component: &FleetComponent,
    network_class_name: &str,
) -> Option<ReplicaSet> {
    let mut updated = rs.clone();
    let rs_spec = updated.spec.get_or_insert_with(ReplicaSetSpec::default);
    let mut changed = false;

    if rs_spec.replicas != Some(component.replicas) {
        rs_spec.replicas = Some(component.replicas);
        changed = true;
    }

    let mut desired_template = component.ship_template.clone().unwrap_or_default();
    inject_fleet_network(&mut desired_template, network_class_name);
    if rs_spec.ship_template != Some(desired_template.clone()) {
        rs_spec.ship_template = Some(desired_template);
        changed = true;
    }

    changed.then_some(updated)
}

async fn create_replicaset_if_absent(
    rs_api: &Api<ReplicaSet>,
    rs: ReplicaSet,
) -> Result<(), ControllerError> {
    match rs_api.create(rs).await {
        Ok(_) => Ok(()),
        Err(tugboat_client::Error::Api(status)) if status.code == 409 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

async fn delete_replicaset_ignore_not_found(
    rs_api: &Api<ReplicaSet>,
    name: &str,
) -> Result<(), ControllerError> {
    match rs_api.delete(name).await {
        Ok(_) => Ok(()),
        Err(tugboat_client::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn fleet_status(owned_replicasets: &[&ReplicaSet], total_components: usize) -> FleetStatus {
    let ready_components = owned_replicasets
        .iter()
        .filter(|rs| {
            rs.status
                .as_ref()
                .map(|status| {
                    let replicas = status.replicas;
                    let ready_replicas = status.ready_replicas;
                    ready_replicas == replicas && replicas > 0
                })
                .unwrap_or(false)
        })
        .count() as i32;

    FleetStatus {
        ready_components,
        total_components: total_components as i32,
    }
}

#[async_trait::async_trait]
impl TugboatController for FleetController {
    fn name(&self) -> &str {
        "fleet"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

impl FleetReconciler {
    async fn reconcile_applied(&self, fleet: Fleet) -> Result<Action, ControllerError> {
        let Some(namespace) = fleet.namespace() else {
            return Err(ControllerError::MissingNamespace("Fleet"));
        };
        let fleet_name = fleet.name().unwrap_or_default().to_string();
        let fleet_spec = fleet
            .spec
            .as_ref()
            .ok_or(ControllerError::MissingFleetSpec {
                namespace: namespace.to_string(),
                name: fleet_name,
            })?;
        let fleet_api: Api<Fleet> = Api::namespaced(self.client.clone(), namespace);
        let network_class_api: Api<ClusterNetworkClass> = Api::all(self.client.clone());

        let Some(_) = network_class_api
            .get(&fleet_spec.network_class_name)
            .await?
        else {
            tracing::debug!(
                "ClusterNetworkClass '{}' referenced by Fleet '{}/{}' is not available yet",
                fleet_spec.network_class_name,
                namespace,
                fleet.name().unwrap_or_default()
            );
            return Ok(Action::requeue(Duration::from_secs(30)));
        };

        let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
        let replicasets = rs_api.list().await?;
        let managed = managed_replicasets(&replicasets, &fleet);

        for component in &fleet_spec.components {
            if let Some(existing_rs) = find_rs_for_component(&managed, &component.name) {
                if let Some(updated_rs) = update_replicaset_for_component(
                    existing_rs,
                    component,
                    &fleet_spec.network_class_name,
                ) {
                    rs_api
                        .replace(existing_rs.name().unwrap_or_default(), updated_rs)
                        .await?;
                }
                continue;
            }

            create_replicaset_if_absent(
                &rs_api,
                build_replicaset_for_component(&fleet, component, &fleet_spec.network_class_name),
            )
            .await?;
        }

        for rs in managed {
            let Some(existing_component_name) = component_name(rs) else {
                continue;
            };
            if fleet_spec
                .components
                .iter()
                .any(|component| component.name == existing_component_name)
            {
                continue;
            }
            if let Some(name) = rs.name() {
                delete_replicaset_ignore_not_found(&rs_api, name).await?;
            }
        }

        let latest_replicasets = rs_api.list().await?;
        let latest_managed = managed_replicasets(&latest_replicasets, &fleet);
        let desired_status = fleet_status(&latest_managed, fleet_spec.components.len());
        if fleet.status.as_ref() != Some(&desired_status) {
            let mut updated_fleet = fleet.clone();
            updated_fleet.status = Some(desired_status);
            let updated_fleet_name = updated_fleet.name().unwrap_or_default().to_string();
            fleet_api
                .replace_status(&updated_fleet_name, updated_fleet)
                .await?;
        }

        Ok(Action::await_change())
    }

    async fn reconcile_deleted(&self, fleet: Fleet) -> Result<Action, ControllerError> {
        let Some(namespace) = fleet.namespace() else {
            return Err(ControllerError::MissingNamespace("Fleet"));
        };
        let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
        let replicasets = rs_api.list().await?;
        for rs in managed_replicasets(&replicasets, &fleet) {
            if let Some(name) = rs.name() {
                delete_replicaset_ignore_not_found(&rs_api, name).await?;
            }
        }
        Ok(Action::await_change())
    }
}

#[async_trait::async_trait]
impl Reconciler<Fleet> for FleetReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<Fleet>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(fleet) => self.reconcile_applied(fleet).await,
            ReconcileEvent::Deleted(fleet) => self.reconcile_deleted(fleet).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FLEET_COMPONENT_LABEL, FLEET_NAME_LABEL, build_replicaset_for_component, component_name,
        find_rs_for_component, fleet_status, inject_fleet_network, is_owned_by_fleet,
        managed_replicasets, owner_reference_for_fleet, update_replicaset_for_component,
    };
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::apps::v1::{
        Fleet, FleetComponent, FleetSpec, FleetStatus, ReplicaSet, ReplicaSetSpec,
        ReplicaSetStatus, ShipTemplateSpec,
    };
    use tugboat_resources::manifests::core::v1::{ShipNetworkClassReference, ShipSpec};
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};

    fn fleet() -> Fleet {
        Fleet {
            object_meta: Some(ObjectMeta {
                name: Some("demo".to_string()),
                namespace: Some("default".to_string()),
                uid: Some("fleet-uid".to_string()),
                ..Default::default()
            }),
            spec: Some(FleetSpec {
                network_class_name: "overlay".to_string(),
                components: vec![],
            }),
            ..Default::default()
        }
    }

    fn component(name: &str, replicas: i32, image: &str) -> FleetComponent {
        FleetComponent {
            name: name.to_string(),
            replicas,
            ship_template: Some(ShipTemplateSpec {
                spec: Some(ShipSpec {
                    image: image.to_string(),
                    ship_class: "standard".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }
    }

    fn managed_rs(component_name: &str) -> ReplicaSet {
        ReplicaSet {
            object_meta: Some(ObjectMeta {
                name: Some(format!("demo-{component_name}")),
                namespace: Some("default".to_string()),
                labels: [
                    (FLEET_NAME_LABEL.to_string(), "demo".to_string()),
                    (
                        FLEET_COMPONENT_LABEL.to_string(),
                        component_name.to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
                owner_references: vec![OwnerReference {
                    api_version: "apps/v1".to_string(),
                    kind: "Fleet".to_string(),
                    name: "demo".to_string(),
                    uid: "fleet-uid".to_string(),
                    controller: Some(true),
                }],
                ..Default::default()
            }),
            spec: Some(ReplicaSetSpec {
                replicas: Some(1),
                selector: [
                    (FLEET_NAME_LABEL.to_string(), "demo".to_string()),
                    (
                        FLEET_COMPONENT_LABEL.to_string(),
                        component_name.to_string(),
                    ),
                ]
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

    #[test]
    fn owner_reference_matches_fleet() {
        let fleet = fleet();

        assert_eq!(
            owner_reference_for_fleet(&fleet),
            OwnerReference {
                api_version: "apps/v1".to_string(),
                kind: "Fleet".to_string(),
                name: "demo".to_string(),
                uid: "fleet-uid".to_string(),
                controller: Some(true),
            }
        );
    }

    #[test]
    fn inject_fleet_network_appends_only_when_missing() {
        let mut template = ShipTemplateSpec {
            spec: Some(ShipSpec {
                network_class_ref: vec![ShipNetworkClassReference {
                    kind: "NetworkClass".to_string(),
                    name: "old".to_string(),
                    api_group: "core".to_string(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        inject_fleet_network(&mut template, "overlay");

        assert_eq!(
            template.spec.unwrap().network_class_ref,
            vec![
                ShipNetworkClassReference {
                    kind: "NetworkClass".to_string(),
                    name: "old".to_string(),
                    api_group: "core".to_string(),
                },
                ShipNetworkClassReference {
                    kind: "ClusterNetworkClass".to_string(),
                    name: "overlay".to_string(),
                    api_group: "core".to_string(),
                }
            ]
        );
    }

    #[test]
    fn inject_fleet_network_is_idempotent() {
        let mut template = ShipTemplateSpec {
            spec: Some(ShipSpec {
                network_class_ref: vec![ShipNetworkClassReference {
                    kind: "ClusterNetworkClass".to_string(),
                    name: "overlay".to_string(),
                    api_group: "core".to_string(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        inject_fleet_network(&mut template, "overlay");

        assert_eq!(template.spec.unwrap().network_class_ref.len(), 1);
    }

    #[test]
    fn build_replicaset_for_component_sets_labels_owner_and_template() {
        let fleet = fleet();
        let component = component("api", 3, "ghcr.io/example/api:v1");

        let rs = build_replicaset_for_component(&fleet, &component, "overlay");
        let meta = rs.object_meta.as_ref().unwrap();
        let spec = rs.spec.as_ref().unwrap();
        let template_spec = spec.ship_template.as_ref().unwrap().spec.as_ref().unwrap();

        assert_eq!(meta.name.as_deref(), Some("demo-api"));
        assert_eq!(meta.namespace.as_deref(), Some("default"));
        assert_eq!(
            meta.labels.get(FLEET_NAME_LABEL).map(String::as_str),
            Some("demo")
        );
        assert_eq!(
            meta.labels.get(FLEET_COMPONENT_LABEL).map(String::as_str),
            Some("api")
        );
        assert_eq!(
            meta.owner_references,
            vec![owner_reference_for_fleet(&fleet)]
        );
        assert_eq!(spec.replicas, Some(3));
        assert_eq!(
            spec.selector.get(FLEET_COMPONENT_LABEL).map(String::as_str),
            Some("api")
        );
        assert_eq!(template_spec.image, "ghcr.io/example/api:v1");
        assert_eq!(
            template_spec.network_class_ref,
            vec![ShipNetworkClassReference {
                kind: "ClusterNetworkClass".to_string(),
                name: "overlay".to_string(),
                api_group: "core".to_string(),
            }]
        );
    }

    #[test]
    fn update_replicaset_for_component_updates_replicas_and_template() {
        let rs = managed_rs("api");
        let component = component("api", 4, "ghcr.io/example/api:v2");

        let updated = update_replicaset_for_component(&rs, &component, "overlay").unwrap();
        let spec = updated.spec.unwrap();
        let template_spec = spec.ship_template.unwrap().spec.unwrap();

        assert_eq!(spec.replicas, Some(4));
        assert_eq!(template_spec.image, "ghcr.io/example/api:v2");
        assert_eq!(
            template_spec.network_class_ref,
            vec![ShipNetworkClassReference {
                kind: "ClusterNetworkClass".to_string(),
                name: "overlay".to_string(),
                api_group: "core".to_string(),
            }]
        );
    }

    #[test]
    fn update_replicaset_for_component_returns_none_when_unchanged() {
        let mut rs = managed_rs("api");
        inject_fleet_network(
            rs.spec.as_mut().unwrap().ship_template.as_mut().unwrap(),
            "overlay",
        );
        let component = component("api", 1, "ghcr.io/example/demo:v1");

        assert!(update_replicaset_for_component(&rs, &component, "overlay").is_none());
    }

    #[test]
    fn managed_replicasets_filters_by_fleet_owner() {
        let fleet = fleet();
        let managed = managed_rs("api");
        let mut foreign = managed_rs("worker");
        foreign.object_meta.as_mut().unwrap().owner_references[0].uid = "other".to_string();

        let replicasets = vec![managed.clone(), foreign];
        let managed = managed_replicasets(&replicasets, &fleet);

        assert_eq!(managed.len(), 1);
        assert_eq!(managed[0].name(), Some("demo-api"));
        assert!(is_owned_by_fleet(managed[0], &fleet));
    }

    #[test]
    fn component_lookup_uses_labels() {
        let rs = managed_rs("api");
        let replicasets = vec![&rs];

        assert_eq!(component_name(&rs), Some("api"));
        assert!(find_rs_for_component(&replicasets, "api").is_some());
        assert!(find_rs_for_component(&replicasets, "worker").is_none());
    }

    #[test]
    fn fleet_status_counts_only_fully_ready_components() {
        let mut ready = managed_rs("api");
        ready.status = Some(ReplicaSetStatus {
            replicas: 2,
            ready_replicas: 2,
        });

        let mut not_ready = managed_rs("worker");
        not_ready.status = Some(ReplicaSetStatus {
            replicas: 3,
            ready_replicas: 1,
        });

        assert_eq!(
            fleet_status(&[&ready, &not_ready], 3),
            FleetStatus {
                ready_components: 1,
                total_components: 3,
            }
        );
    }
}
