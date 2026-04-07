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

use crate::reconciler::volume::{NormalizedVolumeSource, normalized_ship_volumes};
use tugboat_resources::manifests::core::v1::{RuntimeHotplug, ShipActualAllocation, ShipSpec};
use tugboat_vm_runtime_interface::hotplug::{
    VmCpuHotplugConfig, VmHotplugRequest, VmMemoryHotplugConfig,
};
use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig, VmVolumeKind};

#[derive(Debug, Clone)]
pub(crate) struct HotplugBaseline {
    pub current_cpu_cores: u64,
    pub current_memory_bytes: u64,
    pub current_nic_ids: Vec<String>,
    pub current_volume_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct HotplugDesired {
    pub desired_cpu_cores: u64,
    pub desired_memory_bytes: u64,
    pub memory_size: String,
    pub nics_added: Vec<VmNetworkConfig>,
    pub volumes_added: Vec<VmVolumeConfig>,
}

#[derive(Debug, Clone)]
pub(crate) struct HotplugPlan {
    pub hotplug_req: Option<VmHotplugRequest>,
    pub has_unsupported_changes: bool,
    pub actual_allocation: ShipActualAllocation,
}

pub(crate) fn classify_hotplug_changes(
    ship_id: &str,
    old_spec: &ShipSpec,
    new_spec: &ShipSpec,
    baseline: &HotplugBaseline,
    desired: &HotplugDesired,
    hotplug: Option<&RuntimeHotplug>,
    has_unsupported_spec_changes: bool,
) -> HotplugPlan {
    let mut unsupported = has_unsupported_spec_changes;
    let hotplug = hotplug.cloned().unwrap_or_default();

    let mut cpu = None;
    let mut actual_cpu_cores = baseline.current_cpu_cores;
    if desired.desired_cpu_cores != baseline.current_cpu_cores {
        let allowed = if desired.desired_cpu_cores > baseline.current_cpu_cores {
            hotplug.cpu.as_ref().map(|cap| cap.add).unwrap_or(false)
        } else {
            hotplug.cpu.as_ref().map(|cap| cap.remove).unwrap_or(false)
        };
        if allowed {
            cpu = Some(VmCpuHotplugConfig {
                cores: desired.desired_cpu_cores,
            });
            actual_cpu_cores = desired.desired_cpu_cores;
        } else {
            unsupported = true;
        }
    }

    let mut memory = None;
    let mut actual_memory_size = byte_size_string(baseline.current_memory_bytes);
    if desired.desired_memory_bytes != baseline.current_memory_bytes {
        let allowed = if desired.desired_memory_bytes > baseline.current_memory_bytes {
            hotplug.memory.as_ref().map(|cap| cap.add).unwrap_or(false)
        } else {
            hotplug
                .memory
                .as_ref()
                .map(|cap| cap.remove)
                .unwrap_or(false)
        };
        if allowed {
            memory = Some(VmMemoryHotplugConfig {
                size: desired.desired_memory_bytes,
            });
            actual_memory_size = desired.memory_size.clone();
        } else {
            unsupported = true;
        }
    }

    let old_network_keys = old_spec
        .network_class_ref
        .iter()
        .map(network_ref_key)
        .collect::<Vec<_>>();
    let new_network_keys = new_spec
        .network_class_ref
        .iter()
        .map(network_ref_key)
        .collect::<Vec<_>>();
    let mut nics_removed = Vec::new();
    let mut current_nic_ids = baseline.current_nic_ids.clone();
    let removed_network_indexes = removed_indexes(&old_network_keys, &new_network_keys);
    let retained_old_networks = retained_order(&old_network_keys, &new_network_keys);
    let retained_new_networks = retained_order(&new_network_keys, &old_network_keys);
    if retained_old_networks != retained_new_networks {
        unsupported = true;
    } else {
        for index in removed_network_indexes.into_iter().rev() {
            let allowed = hotplug.nic.as_ref().map(|cap| cap.remove).unwrap_or(false);
            if let Some(id) = current_nic_ids.get(index).cloned() {
                if allowed {
                    current_nic_ids.remove(index);
                    nics_removed.push(id);
                } else {
                    unsupported = true;
                }
            } else {
                unsupported = true;
            }
        }
        nics_removed.reverse();
    }

    let added_network_indexes = added_indexes(&old_network_keys, &new_network_keys);
    let mut nics_added = Vec::new();
    if !added_network_indexes.is_empty() {
        let allowed = hotplug.nic.as_ref().map(|cap| cap.add).unwrap_or(false);
        if !allowed || added_network_indexes.len() != desired.nics_added.len() {
            unsupported = true;
        } else {
            for nic in &desired.nics_added {
                current_nic_ids.push(nic_device_id(&nic.mac_address));
            }
            nics_added = desired.nics_added.clone();
        }
    }

    let old_volume_keys = pvc_volume_aliases(old_spec);
    let new_volume_keys = pvc_volume_aliases(new_spec);
    let mut volumes_removed = Vec::new();
    let mut current_volume_ids = baseline.current_volume_ids.clone();
    let removed_volume_indexes = removed_indexes(&old_volume_keys, &new_volume_keys);
    let retained_old_volumes = retained_order(&old_volume_keys, &new_volume_keys);
    let retained_new_volumes = retained_order(&new_volume_keys, &old_volume_keys);
    if retained_old_volumes != retained_new_volumes {
        unsupported = true;
    } else {
        for index in removed_volume_indexes.into_iter().rev() {
            let allowed = hotplug
                .storage
                .as_ref()
                .map(|cap| cap.remove)
                .unwrap_or(false);
            if let Some(id) = current_volume_ids.get(index).cloned() {
                if allowed {
                    current_volume_ids.remove(index);
                    volumes_removed.push(id);
                } else {
                    unsupported = true;
                }
            } else {
                unsupported = true;
            }
        }
        volumes_removed.reverse();
    }

    let added_volume_indexes = added_indexes(&old_volume_keys, &new_volume_keys);
    let mut volumes_added = Vec::new();
    if !added_volume_indexes.is_empty() {
        let allowed = hotplug.storage.as_ref().map(|cap| cap.add).unwrap_or(false);
        let block_only = desired
            .volumes_added
            .iter()
            .all(|volume| volume.kind == VmVolumeKind::Block);
        if !allowed || added_volume_indexes.len() != desired.volumes_added.len() || !block_only {
            unsupported = true;
        } else {
            for volume in &desired.volumes_added {
                current_volume_ids.push(volume_device_id(&volume.host_path));
            }
            volumes_added = desired.volumes_added.clone();
        }
    }

    let hotplug_req = (cpu.is_some()
        || memory.is_some()
        || !nics_added.is_empty()
        || !nics_removed.is_empty()
        || !volumes_added.is_empty()
        || !volumes_removed.is_empty())
    .then(|| VmHotplugRequest {
        id: ship_id.to_string(),
        cpu,
        memory,
        nics_added,
        nics_removed,
        volumes_added,
        volumes_removed,
    });

    HotplugPlan {
        hotplug_req,
        has_unsupported_changes: unsupported,
        actual_allocation: ShipActualAllocation {
            cpu_cores: Some(actual_cpu_cores),
            memory_size: Some(actual_memory_size),
            nic_ids: current_nic_ids,
            volume_ids: current_volume_ids,
        },
    }
}

fn network_ref_key(
    reference: &tugboat_resources::manifests::core::v1::ShipNetworkClassReference,
) -> String {
    format!(
        "{}|{}|{}",
        reference.api_group, reference.kind, reference.name
    )
}

fn pvc_volume_aliases(spec: &ShipSpec) -> Vec<String> {
    normalized_ship_volumes(spec)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|volume| match volume.source {
            NormalizedVolumeSource::PersistentVolumeClaim { .. } => Some(volume.name),
            _ => None,
        })
        .collect()
}

fn added_indexes(old: &[String], new: &[String]) -> Vec<usize> {
    new.iter()
        .enumerate()
        .filter_map(|(index, item)| (!old.contains(item)).then_some(index))
        .collect()
}

fn removed_indexes(old: &[String], new: &[String]) -> Vec<usize> {
    old.iter()
        .enumerate()
        .filter_map(|(index, item)| (!new.contains(item)).then_some(index))
        .collect()
}

fn retained_order(base: &[String], other: &[String]) -> Vec<String> {
    base.iter()
        .filter(|item| other.contains(*item))
        .cloned()
        .collect()
}

fn byte_size_string(size: u64) -> String {
    size.to_string()
}

fn nic_device_id(mac_address: &str) -> String {
    format!("nic-{}", sanitize_identifier(mac_address))
}

fn volume_device_id(host_path: &str) -> String {
    format!("dev-{}", sanitize_identifier(host_path))
}

fn sanitize_identifier(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::{HotplugBaseline, HotplugDesired, classify_hotplug_changes};
    use tugboat_resources::manifests::core::v1::{
        HotplugCapabilities, RuntimeHotplug, ShipNetworkClassReference, ShipSpec,
    };
    use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig};

    fn ship_spec() -> ShipSpec {
        ShipSpec {
            image: "registry.example.com/test:1".to_string(),
            ship_class: "small".to_string(),
            node_name: Some("node-a".to_string()),
            network_class_ref: vec![],
            uefi: None,
            tolerations: vec![],
            scheduler_name: None,
            volume_claim_ref: vec![],
            volumes: vec![],
            target_node_name: None,
            runtime_class: None,
        }
    }

    fn enabled_hotplug() -> RuntimeHotplug {
        RuntimeHotplug {
            cpu: Some(HotplugCapabilities {
                add: true,
                remove: true,
            }),
            memory: Some(HotplugCapabilities {
                add: true,
                remove: true,
            }),
            nic: Some(HotplugCapabilities {
                add: true,
                remove: true,
            }),
            storage: Some(HotplugCapabilities {
                add: true,
                remove: true,
            }),
        }
    }

    #[test]
    fn cpu_increase_with_flag_enabled_is_hotpluggable() {
        let old_spec = ship_spec();
        let new_spec = ship_spec();
        let baseline = HotplugBaseline {
            current_cpu_cores: 2,
            current_memory_bytes: 1024,
            current_nic_ids: vec![],
            current_volume_ids: vec![],
        };
        let desired = HotplugDesired {
            desired_cpu_cores: 4,
            desired_memory_bytes: 1024,
            memory_size: "1024".to_string(),
            nics_added: vec![],
            volumes_added: vec![],
        };

        let plan = classify_hotplug_changes(
            "ship-1",
            &old_spec,
            &new_spec,
            &baseline,
            &desired,
            Some(&enabled_hotplug()),
            false,
        );

        let request = plan.hotplug_req.expect("hotplug request");
        assert_eq!(request.cpu.expect("cpu request").cores, 4);
        assert!(!plan.has_unsupported_changes);
    }

    #[test]
    fn cpu_increase_with_flag_disabled_returns_empty_plan() {
        let old_spec = ship_spec();
        let new_spec = ship_spec();
        let baseline = HotplugBaseline {
            current_cpu_cores: 2,
            current_memory_bytes: 1024,
            current_nic_ids: vec![],
            current_volume_ids: vec![],
        };
        let desired = HotplugDesired {
            desired_cpu_cores: 4,
            desired_memory_bytes: 1024,
            memory_size: "1024".to_string(),
            nics_added: vec![],
            volumes_added: vec![],
        };

        let plan = classify_hotplug_changes(
            "ship-1",
            &old_spec,
            &new_spec,
            &baseline,
            &desired,
            Some(&RuntimeHotplug::default()),
            false,
        );

        assert!(plan.hotplug_req.is_none());
        assert!(plan.has_unsupported_changes);
    }

    #[test]
    fn mixed_hotplug_and_unsupported_change() {
        let old_spec = ship_spec();
        let mut new_spec = ship_spec();
        new_spec.image = "registry.example.com/test:2".to_string();
        new_spec.network_class_ref.push(ShipNetworkClassReference {
            api_group: "core".to_string(),
            kind: "NetworkClass".to_string(),
            name: "frontend".to_string(),
        });
        let baseline = HotplugBaseline {
            current_cpu_cores: 2,
            current_memory_bytes: 1024,
            current_nic_ids: vec![],
            current_volume_ids: vec![],
        };
        let desired = HotplugDesired {
            desired_cpu_cores: 4,
            desired_memory_bytes: 1024,
            memory_size: "1024".to_string(),
            nics_added: vec![VmNetworkConfig {
                iface_name: "eth0".to_string(),
                mac_address: "52:54:00:00:00:01".to_string(),
            }],
            volumes_added: vec![VmVolumeConfig::block("/var/lib/test.img", "raw", false)],
        };

        let plan = classify_hotplug_changes(
            "ship-1",
            &old_spec,
            &new_spec,
            &baseline,
            &desired,
            Some(&enabled_hotplug()),
            true,
        );

        let request = plan.hotplug_req.expect("hotplug request");
        assert_eq!(request.cpu.expect("cpu request").cores, 4);
        assert!(request.nics_added.len() == 1);
        assert!(plan.has_unsupported_changes);
    }
}
