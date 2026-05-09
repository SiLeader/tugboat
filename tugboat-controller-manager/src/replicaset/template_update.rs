use crate::change_classifier::TemplateChangeKind;
use crate::error::ControllerError;
use crate::replicaset::status::ship_is_ready;
use std::time::Duration;
use tugboat_client::Api;
use tugboat_client::runtime::Action;
use tugboat_resources::manifests::apps::v1::ReplicaSet;
use tugboat_resources::manifests::core::v1::{Ship, ShipSpec};
use tugboat_resources::{ObjectMetaResource, ShipMigrationExt};

pub(super) const UPDATE_STRATEGY_ANNOTATION: &str = "tugboat.cloud/update-strategy";
pub(super) const UPDATE_STRATEGY_ALL: &str = "all";

pub(super) fn needs_spec_update(template_spec: &ShipSpec, ship_spec: &ShipSpec) -> bool {
    let mut normalized_ship_spec = ship_spec.clone();
    normalized_ship_spec.node_name = template_spec.node_name.clone();
    normalized_ship_spec.scheduler_name = template_spec.scheduler_name.clone();
    normalized_ship_spec.target_node_name = template_spec.target_node_name.clone();

    template_spec != &normalized_ship_spec
}

pub(super) fn ship_needs_update(ship: &Ship, template_spec: &ShipSpec) -> bool {
    ship.spec
        .as_ref()
        .is_some_and(|ship_spec| needs_spec_update(template_spec, ship_spec))
}

pub(super) fn apply_template_spec(ship: &mut Ship, template_spec: &ShipSpec) {
    let current_spec = ship.spec.clone().unwrap_or_default();
    ship.spec = Some(ShipSpec {
        node_name: current_spec.node_name,
        scheduler_name: current_spec.scheduler_name,
        target_node_name: current_spec.target_node_name,
        ..template_spec.clone()
    });
}

pub(super) fn update_strategy(rs: &ReplicaSet) -> Option<&str> {
    rs.object_meta()
        .as_ref()
        .and_then(|meta| meta.annotations.get(UPDATE_STRATEGY_ANNOTATION))
        .map(String::as_str)
}

pub(super) fn migration_blocks_template_update(
    matching_ships: &[&Ship],
    template_spec: &ShipSpec,
    change_kind: TemplateChangeKind,
) -> bool {
    matches!(
        change_kind,
        TemplateChangeKind::InPlace | TemplateChangeKind::Hotplug
    ) && matching_ships
        .iter()
        .any(|ship| ship_needs_update(ship, template_spec) && ship.has_active_migration())
}

pub(super) fn should_wait_for_ready_before_update(
    matching_ships: &[&Ship],
    template_spec: &ShipSpec,
    change_kind: TemplateChangeKind,
) -> bool {
    matches!(
        change_kind,
        TemplateChangeKind::InPlace | TemplateChangeKind::Hotplug
    ) && matching_ships.iter().any(|ship| {
        ship_needs_update(ship, template_spec)
            && !ship.has_active_migration()
            && !ship_is_ready(ship)
    })
}

pub(super) async fn reconcile_template_updates(
    ship_api: &Api<Ship>,
    rs: &ReplicaSet,
    matching_ships: &[&Ship],
    template_spec: &ShipSpec,
    change_kind: TemplateChangeKind,
    migration_blocks_update: bool,
) -> Result<Option<Action>, ControllerError> {
    match change_kind {
        TemplateChangeKind::NoChange => Ok(None),
        TemplateChangeKind::RequiresRotation => {
            let update_all = update_strategy(rs) == Some(UPDATE_STRATEGY_ALL);
            apply_template_updates(ship_api, matching_ships, template_spec, true, update_all).await
        }
        TemplateChangeKind::InPlace | TemplateChangeKind::Hotplug => {
            let update_all = update_strategy(rs) == Some(UPDATE_STRATEGY_ALL)
                || change_kind == TemplateChangeKind::Hotplug;
            let action =
                apply_template_updates(ship_api, matching_ships, template_spec, false, update_all)
                    .await?;
            if action.is_none() && migration_blocks_update {
                return Ok(Some(Action::requeue(Duration::from_secs(5))));
            }
            Ok(action)
        }
    }
}

async fn apply_template_updates(
    ship_api: &Api<Ship>,
    matching_ships: &[&Ship],
    template_spec: &ShipSpec,
    replace: bool,
    update_all: bool,
) -> Result<Option<Action>, ControllerError> {
    let mut updated_any = false;
    for ship in matching_ships {
        if ship.has_active_migration() || !ship_needs_update(ship, template_spec) {
            continue;
        }
        let Some(name) = ship.name() else {
            continue;
        };

        let mut updated = (*ship).clone();
        apply_template_spec(&mut updated, template_spec);
        if replace {
            ship_api.replace(name, updated).await?;
        } else {
            ship_api.patch(name, &updated).await?;
        }
        updated_any = true;

        if !update_all {
            break;
        }
    }

    if updated_any {
        return Ok(Some(Action::requeue(Duration::from_secs(5))));
    }
    Ok(None)
}
