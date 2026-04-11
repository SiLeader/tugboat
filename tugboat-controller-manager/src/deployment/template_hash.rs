use crate::error::ControllerError;
use serde_json::{Value, to_string};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::apps::v1::{Deployment, ReplicaSet, ShipTemplateSpec};

const TEMPLATE_HASH_BYTES: usize = 8;

pub(super) fn template_hash(template: &ShipTemplateSpec) -> String {
    let json = canonical_json_string(template);
    let hash = Sha256::digest(json.as_bytes());
    hash[..TEMPLATE_HASH_BYTES]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

pub(super) fn replicaset_has_template_hash(rs: &ReplicaSet, hash: &str) -> bool {
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

pub(super) fn deployment_template(dep: &Deployment) -> Result<&ShipTemplateSpec, ControllerError> {
    let namespace = dep.namespace().unwrap_or_default().to_string();
    let name = dep.name().unwrap_or_default().to_string();

    dep.spec
        .as_ref()
        .ok_or_else(|| ControllerError::MissingDeploymentSpec {
            namespace: namespace.clone(),
            name: name.clone(),
        })?
        .ship_template
        .as_ref()
        .ok_or(ControllerError::MissingDeploymentTemplate { namespace, name })
}

pub(super) fn deployment_template_hash(dep: &Deployment) -> Result<String, ControllerError> {
    Ok(template_hash(deployment_template(dep)?))
}
