use crate::error::ControllerError;
use crate::replicaset::ship_builder::build_ship;
use crate::workload::{creation_sort_key, matches_selector};
use std::collections::HashMap;
use std::time::Duration;
use tracing::debug;
use tugboat_client::Api;
use tugboat_client::runtime::Action;
use tugboat_resources::manifests::apps::v1::{ReplicaSet, ReplicaSetSpec};
use tugboat_resources::manifests::core::v1::Ship;
use tugboat_resources::{ObjectMetaResource, ShipMigrationExt};

pub(super) fn ship_matches_selector(ship: &Ship, selector: &HashMap<String, String>) -> bool {
    ship.object_meta
        .as_ref()
        .is_some_and(|meta| matches_selector(&meta.labels, selector))
}

pub(super) fn owned_ships<'a>(
    ships: &'a [Ship],
    selector: &HashMap<String, String>,
) -> Vec<&'a Ship> {
    ships
        .iter()
        .filter(|ship| ship_matches_selector(ship, selector))
        .collect()
}

pub(super) fn is_owned_by_replicaset(ship: &Ship, rs: &ReplicaSet) -> bool {
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

pub(super) fn ship_creation_sort_key(ship: &Ship) -> (i64, i32, String) {
    creation_sort_key(ship.object_meta.as_ref(), ship.name())
}

pub(super) fn excess_ships_to_delete<'a>(
    owned_ships: &[&'a Ship],
    desired: usize,
) -> Vec<&'a Ship> {
    if owned_ships.len() <= desired {
        return Vec::new();
    }

    let mut sorted = owned_ships.to_vec();
    sorted.retain(|ship| !ship.has_active_migration());
    sorted.sort_by_key(|ship| std::cmp::Reverse(ship_creation_sort_key(ship)));
    sorted.truncate((owned_ships.len() - desired).min(sorted.len()));
    sorted
}

pub(super) async fn delete_ship_ignore_not_found(
    ship_api: &Api<Ship>,
    name: &str,
) -> Result<(), ControllerError> {
    match ship_api.delete(name).await {
        Ok(_) => Ok(()),
        Err(tugboat_client::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub(super) async fn reconcile_ship_count(
    ship_api: &Api<Ship>,
    rs: &ReplicaSet,
    rs_spec: &ReplicaSetSpec,
    matching_ships: &[&Ship],
    desired: usize,
) -> Result<Option<Action>, ControllerError> {
    for _ in matching_ships.len()..desired {
        let ship = build_ship(rs, &rs_spec.ship_template.clone().unwrap_or_default());
        debug!(
            rs = rs.name().unwrap_or_default(),
            ship = ship.name().unwrap_or_default(),
            "creating ship for replicaset"
        );
        match ship_api.create(ship).await {
            Ok(_) => {}
            Err(tugboat_client::Error::Api(status)) if status.code == 409 => {}
            Err(err) => return Err(err.into()),
        }
    }

    if matching_ships.len() <= desired {
        return Ok(None);
    }

    let excess = matching_ships.len() - desired;
    let to_delete = excess_ships_to_delete(matching_ships, desired);
    for ship in &to_delete {
        if let Some(name) = ship.name() {
            delete_ship_ignore_not_found(ship_api, name).await?;
        }
    }
    if to_delete.len() < excess {
        return Ok(Some(Action::requeue(Duration::from_secs(5))));
    }

    Ok(None)
}
