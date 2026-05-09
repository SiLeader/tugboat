use crate::workload::owner_reference_for;
use rand::{RngExt, rng};
use tugboat_resources::manifests::apps::v1::{ReplicaSet, ShipTemplateSpec};
use tugboat_resources::manifests::core::v1::Ship;
use tugboat_resources::manifests::meta::v1::OwnerReference;
use tugboat_resources::{ObjectMetaResource, Resource, SetTypeMeta};

pub(super) fn generate_suffix() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rng();
    (0..5)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

pub(super) fn owner_reference_for_replicaset(rs: &ReplicaSet) -> OwnerReference {
    owner_reference_for(rs)
}

pub(super) fn build_ship(rs: &ReplicaSet, template: &ShipTemplateSpec) -> Ship {
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
