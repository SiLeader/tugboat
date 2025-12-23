use crate::manifests::object_meta::ObjectMeta;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ship {
    pub metadata: ObjectMeta,
    pub spec: ShipSpec,
    pub status: ShipStatus,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipSpec {
    pub image: String,
    pub machine: String,
    pub nics: Vec<Nic>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipStatus {
    pub associated_persistent_volume: HashMap<String, String>,
    pub nics: Vec<Nic>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Nic {
    pub network_group: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub mac: Option<String>,
}
