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
use sha2::Digest;

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

pub fn sanitize_identifier(value: &str) -> String {
    let digest = sha2::Sha256::digest(value.as_bytes());
    digest
        .into_iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

pub fn normalize_identifier_key(value: &str, prefixes: &[&str]) -> String {
    let trimmed = prefixes
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .unwrap_or(value)
        .trim();
    if is_sanitized_identifier(trimmed) {
        trimmed.to_ascii_lowercase()
    } else {
        sanitize_identifier(trimmed)
    }
}

fn is_sanitized_identifier(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{
        VmCpuHotplugConfig, VmHotplugRequest, VmMemoryHotplugConfig, normalize_identifier_key,
        sanitize_identifier,
    };
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

    #[test]
    fn sanitize_identifier_creates_stable_sha256_keys() {
        let disk1 = sanitize_identifier("/var/lib/disk1.img");
        let disk1_again = sanitize_identifier("/var/lib/disk1.img");
        let disk2 = sanitize_identifier("/var/lib/disk2.img");

        assert_eq!(disk1, disk1_again);
        assert_ne!(disk1, disk2);
        assert_eq!(disk1.len(), 64);
        assert!(disk1.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn normalize_identifier_key_preserves_existing_hashed_keys() {
        let key = sanitize_identifier("/var/lib/disk1.img");

        assert_eq!(
            normalize_identifier_key(&format!("dev-{key}"), &["dev-", "blk-"]),
            key
        );
        assert_eq!(
            normalize_identifier_key("/var/lib/disk1.img", &["dev-", "blk-"]),
            key
        );
    }
}
