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

use serde::{Deserialize, Serialize};

pub const DEFAULT_MOUNT_NAMESPACE_DIR: &str = "/var/run/tugboat/mntns";

pub fn mount_namespace_path(id: &str) -> String {
    format!("{DEFAULT_MOUNT_NAMESPACE_DIR}/{id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmRunRequest {
    pub image: String,
    #[serde(default = "default_vm_disk_image_format")]
    pub image_format: VmDiskImageFormat,
    pub cpu: VmCpuConfig,
    pub memory: VmMemoryConfig,
    pub id: String,
    pub networks: Vec<VmNetworkConfig>,
    pub volumes: Vec<VmVolumeConfig>,
    pub uefi: VmUefiConfig,
    #[serde(default)]
    pub incoming: Option<VmIncomingMigrationConfig>,
    #[serde(default)]
    pub restore_handle: Option<String>,
    #[serde(default)]
    pub restore_source_id: Option<String>,
    #[serde(default)]
    pub user: VmExecUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmDiskImageFormat {
    Qcow2,
    Raw,
}

fn default_vm_disk_image_format() -> VmDiskImageFormat {
    VmDiskImageFormat::Qcow2
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmExecUser {
    pub user: Option<u32>,
    pub group: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmCpuConfig {
    pub architecture: String,
    pub cores: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmMemoryConfig {
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmIncomingMigrationConfig {
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmNetworkConfig {
    pub iface_name: String,
    pub mac_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmUefiConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmVolumeConfig {
    pub host_path: String,
    #[serde(default)]
    pub kind: VmVolumeKind,
    #[serde(default = "default_vm_volume_format")]
    pub format: String,
    pub read_only: bool,
    #[serde(default)]
    pub mount_tag: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VmVolumeKind {
    #[default]
    Block,
    Filesystem,
}

impl VmVolumeConfig {
    pub fn block(host_path: impl Into<String>, format: impl Into<String>, read_only: bool) -> Self {
        Self {
            host_path: host_path.into(),
            kind: VmVolumeKind::Block,
            format: format.into(),
            read_only,
            mount_tag: String::new(),
        }
    }

    pub fn filesystem(
        host_path: impl Into<String>,
        mount_tag: impl Into<String>,
        read_only: bool,
    ) -> Self {
        Self {
            host_path: host_path.into(),
            kind: VmVolumeKind::Filesystem,
            format: default_vm_volume_format(),
            read_only,
            mount_tag: mount_tag.into(),
        }
    }
}

fn default_vm_volume_format() -> String {
    "raw".to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        VmCpuConfig, VmDiskImageFormat, VmExecUser, VmMemoryConfig, VmRunRequest, VmUefiConfig,
    };
    use super::{VmVolumeConfig, mount_namespace_path};

    #[test]
    fn can_build_mount_namespace_path() {
        assert_eq!(
            mount_namespace_path("ship-uid"),
            "/var/run/tugboat/mntns/ship-uid"
        );
    }

    #[test]
    fn vm_run_request_serializes_image_format() {
        let request = VmRunRequest {
            image: "/images/disk.raw".to_string(),
            image_format: VmDiskImageFormat::Raw,
            cpu: VmCpuConfig {
                architecture: "x86_64".to_string(),
                cores: 2,
            },
            memory: VmMemoryConfig { size: 1024 },
            id: "ship-uid".to_string(),
            networks: Vec::new(),
            volumes: vec![VmVolumeConfig::block("/data/disk.raw", "raw", true)],
            uefi: VmUefiConfig { enabled: false },
            incoming: None,
            restore_handle: None,
            restore_source_id: None,
            user: VmExecUser::default(),
        };

        let value = serde_json::to_value(request).expect("serialize");

        assert_eq!(value["imageFormat"], "raw");
    }

    #[test]
    fn vm_run_request_defaults_missing_image_format_to_qcow2() {
        let request: VmRunRequest = serde_json::from_value(serde_json::json!({
            "image": "/images/disk.qcow2",
            "cpu": {
                "architecture": "x86_64",
                "cores": 2
            },
            "memory": {
                "size": 1024
            },
            "id": "ship-uid",
            "networks": [],
            "volumes": [],
            "uefi": {
                "enabled": false
            }
        }))
        .expect("deserialize");

        assert_eq!(request.image_format, VmDiskImageFormat::Qcow2);
    }
}
