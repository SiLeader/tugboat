use tugboat_resources::manifests::apps::v1::ReplicaSetStatus;
use tugboat_resources::manifests::core::v1::Ship;

pub(super) fn ship_is_ready(ship: &Ship) -> bool {
    ship.status.as_ref().is_some_and(|status| {
        status
            .conditions
            .iter()
            .any(|condition| condition.status == "Running")
    })
}

pub(super) fn build_replicaset_status(owned_ships: &[&Ship]) -> ReplicaSetStatus {
    ReplicaSetStatus {
        replicas: owned_ships.len() as i32,
        ready_replicas: owned_ships
            .iter()
            .filter(|ship| ship_is_ready(ship))
            .count() as i32,
    }
}
