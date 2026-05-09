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
use std::{fmt, path::Path};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotplugValidationError {
    field: &'static str,
    message: String,
}

impl HotplugValidationError {
    fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }

    pub fn field(&self) -> &'static str {
        self.field
    }
}

impl fmt::Display for HotplugValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for HotplugValidationError {}

impl VmHotplugRequest {
    pub fn validate_and_normalize(self) -> Result<Self, HotplugValidationError> {
        validate_and_normalize_hotplug_request(self)
    }
}

pub fn validate_and_normalize_hotplug_request(
    mut request: VmHotplugRequest,
) -> Result<VmHotplugRequest, HotplugValidationError> {
    validate_safe_id(&request.id, "id")?;

    for (index, network) in request.nics_added.iter().enumerate() {
        validate_non_empty(
            &network.iface_name,
            "nicsAdded[].ifaceName",
            format!("nicsAdded[{index}].ifaceName must not be empty"),
        )?;
        validate_non_empty(
            &network.mac_address,
            "nicsAdded[].macAddress",
            format!("nicsAdded[{index}].macAddress must not be empty"),
        )?;
    }

    for id in &mut request.nics_removed {
        *id = normalize_hotplug_nic_id(id)?;
    }

    for (index, volume) in request.volumes_added.iter().enumerate() {
        validate_added_block_volume(volume, index)?;
    }

    for id in &mut request.volumes_removed {
        *id = normalize_hotplug_volume_id(id)?;
    }

    Ok(request)
}

pub fn normalize_hotplug_nic_id(id: &str) -> Result<String, HotplugValidationError> {
    let key = validate_removal_identifier_key(id, "nicsRemoved[]", &["nic-", "net-"])?;
    Ok(format!("nic-{key}"))
}

pub fn normalize_hotplug_volume_id(id: &str) -> Result<String, HotplugValidationError> {
    let key = validate_removal_identifier_key(id, "volumesRemoved[]", &["dev-", "blk-"])?;
    Ok(format!("dev-{key}"))
}

pub fn sanitize_identifier(value: &str) -> String {
    let digest = sha2::Sha256::digest(value.as_bytes());
    digest
        .into_iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

pub fn normalize_identifier_key(value: &str, prefixes: &[&str]) -> String {
    let value = value.trim();
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

fn validate_added_block_volume(
    volume: &VmVolumeConfig,
    index: usize,
) -> Result<(), HotplugValidationError> {
    if volume.kind != crate::run::VmVolumeKind::Block {
        return Err(HotplugValidationError::new(
            "volumesAdded[].kind",
            format!("volumesAdded[{index}].kind must be block for hotplug"),
        ));
    }
    if !Path::new(&volume.host_path).is_absolute() {
        return Err(HotplugValidationError::new(
            "volumesAdded[].hostPath",
            format!("volumesAdded[{index}].hostPath must be absolute"),
        ));
    }
    if volume.format != "raw" {
        return Err(HotplugValidationError::new(
            "volumesAdded[].format",
            format!("volumesAdded[{index}].format must be raw for hotplug"),
        ));
    }
    Ok(())
}

fn validate_removal_identifier_key(
    id: &str,
    field: &'static str,
    prefixes: &[&str],
) -> Result<String, HotplugValidationError> {
    let trimmed = prefixes
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id)
        .trim();

    if trimmed.is_empty() {
        return Err(HotplugValidationError::new(
            field,
            "device id must not be empty",
        ));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(HotplugValidationError::new(
            field,
            "device id must not be a path",
        ));
    }
    if trimmed.starts_with('.') || trimmed.contains("..") {
        return Err(HotplugValidationError::new(
            field,
            "device id must not contain path traversal",
        ));
    }

    Ok(normalize_identifier_key(id, prefixes))
}

fn validate_safe_id(id: &str, field: &'static str) -> Result<(), HotplugValidationError> {
    if id.is_empty() {
        return Err(HotplugValidationError::new(field, "must not be empty"));
    }
    if id.starts_with('.') {
        return Err(HotplugValidationError::new(
            field,
            "must not start with '.'",
        ));
    }
    if id.contains("..") {
        return Err(HotplugValidationError::new(field, "must not contain '..'"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(HotplugValidationError::new(
            field,
            "contains invalid characters; only alphanumeric, '-', '_', and '.' are allowed",
        ));
    }
    Ok(())
}

fn validate_non_empty(
    value: &str,
    field: &'static str,
    message: String,
) -> Result<(), HotplugValidationError> {
    if value.is_empty() {
        return Err(HotplugValidationError::new(field, message));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        VmCpuHotplugConfig, VmHotplugRequest, VmMemoryHotplugConfig, normalize_hotplug_nic_id,
        normalize_hotplug_volume_id, normalize_identifier_key, sanitize_identifier,
        validate_and_normalize_hotplug_request,
    };
    use crate::run::{VmNetworkConfig, VmVolumeConfig, VmVolumeKind};

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
        assert_eq!(
            normalize_identifier_key(&format!(" dev-{key} "), &["dev-", "blk-"]),
            key
        );
    }

    #[test]
    fn hotplug_validation_normalizes_removed_device_ids() {
        let nic_key = sanitize_identifier("52:54:00:00:00:01");
        let volume_key = sanitize_identifier("/var/lib/vm/disk1.img");
        let request = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![format!("net-{nic_key}")],
            volumes_added: vec![],
            volumes_removed: vec![format!("blk-{volume_key}")],
        };

        let normalized = validate_and_normalize_hotplug_request(request).unwrap();

        assert_eq!(normalized.nics_removed, vec![format!("nic-{nic_key}")]);
        assert_eq!(
            normalized.volumes_removed,
            vec![format!("dev-{volume_key}")]
        );
    }

    #[test]
    fn hotplug_validation_accepts_absolute_raw_block_volume_and_preserves_readonly() {
        let request = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![],
            volumes_added: vec![VmVolumeConfig::block("/var/lib/vm/disk1.img", "raw", true)],
            volumes_removed: vec![],
        };

        let normalized = request.validate_and_normalize().unwrap();

        assert!(normalized.volumes_added[0].read_only);
    }

    #[test]
    fn hotplug_validation_rejects_invalid_vm_id() {
        let request = VmHotplugRequest {
            id: "../test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![],
            volumes_added: vec![],
            volumes_removed: vec![],
        };

        let error = validate_and_normalize_hotplug_request(request).unwrap_err();

        assert_eq!(error.field(), "id");
    }

    #[test]
    fn hotplug_validation_rejects_relative_block_volume_path() {
        let request = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![],
            volumes_added: vec![VmVolumeConfig::block("disk1.img", "raw", false)],
            volumes_removed: vec![],
        };

        let error = validate_and_normalize_hotplug_request(request).unwrap_err();

        assert_eq!(error.field(), "volumesAdded[].hostPath");
    }

    #[test]
    fn hotplug_validation_rejects_filesystem_volume_hotplug() {
        let request = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![],
            volumes_added: vec![VmVolumeConfig {
                host_path: "/shared".to_string(),
                kind: VmVolumeKind::Filesystem,
                format: "raw".to_string(),
                read_only: false,
                mount_tag: "shared".to_string(),
            }],
            volumes_removed: vec![],
        };

        let error = validate_and_normalize_hotplug_request(request).unwrap_err();

        assert_eq!(error.field(), "volumesAdded[].kind");
    }

    #[test]
    fn hotplug_validation_rejects_non_raw_block_volume_hotplug() {
        let request = VmHotplugRequest {
            id: "test-vm".to_string(),
            cpu: None,
            memory: None,
            nics_added: vec![],
            nics_removed: vec![],
            volumes_added: vec![VmVolumeConfig::block(
                "/var/lib/vm/disk1.qcow2",
                "qcow2",
                false,
            )],
            volumes_removed: vec![],
        };

        let error = validate_and_normalize_hotplug_request(request).unwrap_err();

        assert_eq!(error.field(), "volumesAdded[].format");
    }

    #[test]
    fn hotplug_validation_rejects_path_like_removed_volume_id() {
        let error = normalize_hotplug_volume_id("/var/lib/vm/disk1.img").unwrap_err();

        assert_eq!(error.field(), "volumesRemoved[]");
    }

    #[test]
    fn hotplug_validation_normalizes_unprefixed_removed_ids() {
        let nic_key = normalize_hotplug_nic_id("52:54:00:00:00:01").unwrap();
        let volume_key = normalize_hotplug_volume_id("data-disk").unwrap();

        assert!(nic_key.starts_with("nic-"));
        assert!(volume_key.starts_with("dev-"));
    }
}
