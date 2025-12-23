use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectMeta {
    pub name: Option<String>,
    pub generate_name: Option<String>,
    pub namespace: Option<String>,
    pub resource_id: Option<String>,
    pub resource_version: Option<u64>,
    pub finalizers: Vec<String>,
}
