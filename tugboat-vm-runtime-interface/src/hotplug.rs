// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::run::{VmNetworkConfig, VmVolumeConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmHotplugRequest {
    pub id: String,
    pub cpu: Option<VmCpuHotplugConfig>,
    pub memory: Option<VmMemoryHotplugConfig>,
    pub nics_added: Vec<VmNetworkConfig>,
    pub nics_removed: Vec<String>,
    pub volumes_added: Vec<VmVolumeConfig>,
    pub volumes_removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmCpuHotplugConfig {
    pub cores: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmMemoryHotplugConfig {
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::{VmCpuHotplugConfig, VmHotplugRequest, VmMemoryHotplugConfig};
    use crate::run::{VmNetworkConfig, VmVolumeConfig};

    #[test]
    fn serialize_round_trip() {
        let req = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: Some(VmCpuHotplugConfig { cores: 4 }),
            memory: Some(VmMemoryHotplugConfig {
                size: 8 * 1024 * 1024 * 1024,
            }),
            nics_added: vec![VmNetworkConfig {
                iface_name: "tap0".to_string(),
                mac_address: "02:00:00:00:00:01".to_string(),
            }],
            nics_removed: vec!["net-02:00:00:00:00:02".to_string()],
            volumes_added: vec![VmVolumeConfig::block("/var/lib/vm/disk1.img", "raw", false)],
            volumes_removed: vec!["disk1".to_string()],
        };

        let json = serde_json::to_string(&req).unwrap();
        let decoded: VmHotplugRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.id, "test-vm");
        assert_eq!(decoded.cpu.unwrap().cores, 4);
        assert_eq!(decoded.memory.unwrap().size, 8 * 1024 * 1024 * 1024);
        assert_eq!(decoded.nics_added.len(), 1);
        assert_eq!(decoded.nics_removed, vec!["net-02:00:00:00:00:02"]);
        assert_eq!(decoded.volumes_added.len(), 1);
        assert_eq!(decoded.volumes_removed, vec!["disk1"]);
    }

    #[test]
    fn serialize_round_trip_without_optional_resources() {
        let req = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![],
            volumes_added: vec![],
            volumes_removed: vec![],
        };

        let json = serde_json::to_string(&req).unwrap();
        let decoded: VmHotplugRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.id, "test-vm");
        assert!(decoded.cpu.is_none());
        assert!(decoded.memory.is_none());
        assert!(decoded.nics_added.is_empty());
        assert!(decoded.nics_removed.is_empty());
        assert!(decoded.volumes_added.is_empty());
        assert!(decoded.volumes_removed.is_empty());
    }
}
