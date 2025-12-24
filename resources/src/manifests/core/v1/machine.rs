use crate::manifests::ObjectMeta;
use crate::manifests::sized::SizedString;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipClass {
    pub metadata: ObjectMeta,
    pub spec: ShipClassSpec,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipClassSpec {
    pub cpu: CpuSpec,
    pub memory: MemorySpec,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct MemorySpec(pub SizedString);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuSpec {
    pub architecture: String,
    pub cores: usize,
    pub sockets: usize,
    pub dies: usize,
    pub threads_per_core: usize,
}

impl Default for CpuSpec {
    fn default() -> Self {
        Self {
            architecture: "amd64".to_owned(),
            cores: 1,
            sockets: 1,
            dies: 1,
            threads_per_core: 1,
        }
    }
}
