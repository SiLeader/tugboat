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
    VmCpuHotplugConfig, VmHotplugRequest, VmMemoryHotplugConfig, sanitize_identifier,
};
use tugboat_vm_runtime_interface::run::{VmNetworkConfig, VmVolumeConfig, VmVolumeKind};

#[derive(Clone)]
pub(crate) struct HotplugBaseline {
    pub current_cpu_cores: u64,
    pub current_memory_bytes: u64,
    pub current_nic_ids: Vec<String>,
    pub current_volume_ids: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct HotplugDesired {
    pub desired_cpu_cores: u64,
    pub desired_memory_bytes: u64,
    pub memory_size: String,
    pub nics_added: Vec<VmNetworkConfig>,
    pub volumes_added: Vec<VmVolumeConfig>,
}

#[derive(Clone)]
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

    let cpu = classify_cpu(baseline, desired, &hotplug, &mut unsupported);
    let memory = classify_memory(baseline, desired, &hotplug, &mut unsupported);
    let networks = classify_networks(
        old_spec,
        new_spec,
        baseline,
        desired,
        &hotplug,
        &mut unsupported,
    );
    let volumes = classify_volumes(
        old_spec,
        new_spec,
        baseline,
        desired,
        &hotplug,
        &mut unsupported,
    );

    let hotplug_req = (cpu.config.is_some()
        || memory.config.is_some()
        || !networks.added.is_empty()
        || !networks.removed.is_empty()
        || !volumes.added.is_empty()
        || !volumes.removed.is_empty())
    .then(|| VmHotplugRequest {
        id: ship_id.to_string(),
        cpu: cpu.config,
        memory: memory.config,
        nics_added: networks.added,
        nics_removed: networks.removed,
        volumes_added: volumes.added,
        volumes_removed: volumes.removed,
    });

    HotplugPlan {
        hotplug_req,
        has_unsupported_changes: unsupported,
        actual_allocation: ShipActualAllocation {
            cpu_cores: Some(cpu.actual_cores),
            memory_size: Some(memory.actual_size),
            nic_ids: networks.current_ids,
            volume_ids: volumes.current_ids,
        },
    }
}

struct CpuClassification {
    config: Option<VmCpuHotplugConfig>,
    actual_cores: u64,
}

fn classify_cpu(
    baseline: &HotplugBaseline,
    desired: &HotplugDesired,
    hotplug: &RuntimeHotplug,
    unsupported: &mut bool,
) -> CpuClassification {
    if desired.desired_cpu_cores == baseline.current_cpu_cores {
        return CpuClassification {
            config: None,
            actual_cores: baseline.current_cpu_cores,
        };
    }
    let allowed = if desired.desired_cpu_cores > baseline.current_cpu_cores {
        hotplug.cpu.as_ref().map(|cap| cap.add).unwrap_or(false)
    } else {
        hotplug.cpu.as_ref().map(|cap| cap.remove).unwrap_or(false)
    };
    if allowed {
        CpuClassification {
            config: Some(VmCpuHotplugConfig {
                cores: desired.desired_cpu_cores,
            }),
            actual_cores: desired.desired_cpu_cores,
        }
    } else {
        *unsupported = true;
        CpuClassification {
            config: None,
            actual_cores: baseline.current_cpu_cores,
        }
    }
}

struct MemoryClassification {
    config: Option<VmMemoryHotplugConfig>,
    actual_size: String,
}

fn classify_memory(
    baseline: &HotplugBaseline,
    desired: &HotplugDesired,
    hotplug: &RuntimeHotplug,
    unsupported: &mut bool,
) -> MemoryClassification {
    if desired.desired_memory_bytes == baseline.current_memory_bytes {
        return MemoryClassification {
            config: None,
            actual_size: byte_size_string(baseline.current_memory_bytes),
        };
    }
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
        MemoryClassification {
            config: Some(VmMemoryHotplugConfig {
                size: desired.desired_memory_bytes,
            }),
            actual_size: desired.memory_size.clone(),
        }
    } else {
        *unsupported = true;
        MemoryClassification {
            config: None,
            actual_size: byte_size_string(baseline.current_memory_bytes),
        }
    }
}

struct NetworkClassification {
    added: Vec<VmNetworkConfig>,
    removed: Vec<String>,
    current_ids: Vec<String>,
}

fn classify_networks(
    old_spec: &ShipSpec,
    new_spec: &ShipSpec,
    baseline: &HotplugBaseline,
    desired: &HotplugDesired,
    hotplug: &RuntimeHotplug,
    unsupported: &mut bool,
) -> NetworkClassification {
    let old_keys = old_spec
        .network_class_ref
        .iter()
        .map(network_ref_key)
        .collect::<Vec<_>>();
    let new_keys = new_spec
        .network_class_ref
        .iter()
        .map(network_ref_key)
        .collect::<Vec<_>>();

    let mut current_ids = baseline.current_nic_ids.clone();
    let mut removed = Vec::new();

    if retained_order(&old_keys, &new_keys) != retained_order(&new_keys, &old_keys) {
        *unsupported = true;
    } else {
        for index in removed_indexes(&old_keys, &new_keys).into_iter().rev() {
            let allowed = hotplug.nic.as_ref().map(|cap| cap.remove).unwrap_or(false);
            match (current_ids.get(index).cloned(), allowed) {
                (Some(id), true) => {
                    current_ids.remove(index);
                    removed.push(id);
                }
                _ => *unsupported = true,
            }
        }
        removed.reverse();
    }

    let added_indexes = added_indexes(&old_keys, &new_keys);
    let mut added = Vec::new();
    if !added_indexes.is_empty() {
        let allowed = hotplug.nic.as_ref().map(|cap| cap.add).unwrap_or(false);
        if !allowed || added_indexes.len() != desired.nics_added.len() {
            *unsupported = true;
        } else {
            for nic in &desired.nics_added {
                current_ids.push(nic_device_id(&nic.mac_address));
            }
            added = desired.nics_added.clone();
        }
    }

    NetworkClassification {
        added,
        removed,
        current_ids,
    }
}

struct VolumeClassification {
    added: Vec<VmVolumeConfig>,
    removed: Vec<String>,
    current_ids: Vec<String>,
}

fn classify_volumes(
    old_spec: &ShipSpec,
    new_spec: &ShipSpec,
    baseline: &HotplugBaseline,
    desired: &HotplugDesired,
    hotplug: &RuntimeHotplug,
    unsupported: &mut bool,
) -> VolumeClassification {
    let old_keys = pvc_volume_aliases(old_spec);
    let new_keys = pvc_volume_aliases(new_spec);

    let mut current_ids = baseline.current_volume_ids.clone();
    let mut removed = Vec::new();

    if retained_order(&old_keys, &new_keys) != retained_order(&new_keys, &old_keys) {
        *unsupported = true;
    } else {
        for index in removed_indexes(&old_keys, &new_keys).into_iter().rev() {
            let allowed = hotplug
                .storage
                .as_ref()
                .map(|cap| cap.remove)
                .unwrap_or(false);
            match (current_ids.get(index).cloned(), allowed) {
                (Some(id), true) => {
                    current_ids.remove(index);
                    removed.push(id);
                }
                _ => *unsupported = true,
            }
        }
        removed.reverse();
    }

    let added_indexes = added_indexes(&old_keys, &new_keys);
    let mut added = Vec::new();
    if !added_indexes.is_empty() {
        let allowed = hotplug.storage.as_ref().map(|cap| cap.add).unwrap_or(false);
        let block_only = desired
            .volumes_added
            .iter()
            .all(|volume| volume.kind == VmVolumeKind::Block);
        if !allowed || added_indexes.len() != desired.volumes_added.len() || !block_only {
            *unsupported = true;
        } else {
            for volume in &desired.volumes_added {
                current_ids.push(volume_device_id(&volume.host_path));
            }
            added = desired.volumes_added.clone();
        }
    }

    VolumeClassification {
        added,
        removed,
        current_ids,
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

impl std::fmt::Debug for HotplugBaseline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotplugBaseline")
            .field("current_cpu_cores", &self.current_cpu_cores)
            .field("current_memory_bytes", &self.current_memory_bytes)
            .field("current_nic_ids_len", &self.current_nic_ids.len())
            .field("current_volume_ids_len", &self.current_volume_ids.len())
            .finish()
    }
}

impl std::fmt::Debug for HotplugDesired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotplugDesired")
            .field("desired_cpu_cores", &self.desired_cpu_cores)
            .field("desired_memory_bytes", &self.desired_memory_bytes)
            .field("memory_size", &self.memory_size)
            .field("nics_added_len", &self.nics_added.len())
            .field("volumes_added_len", &self.volumes_added.len())
            .finish()
    }
}

impl std::fmt::Debug for HotplugPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only expose high-level info about the hotplug request and redact IDs.
        let (nics_added_len, nics_removed_len, volumes_added_len, volumes_removed_len) =
            if let Some(req) = &self.hotplug_req {
                (
                    req.nics_added.len(),
                    req.nics_removed.len(),
                    req.volumes_added.len(),
                    req.volumes_removed.len(),
                )
            } else {
                (0, 0, 0, 0)
            };
        f.debug_struct("HotplugPlan")
            .field("has_hotplug_req", &self.hotplug_req.is_some())
            .field("has_unsupported_changes", &self.has_unsupported_changes)
            .field("hotplug_req_nics_added_len", &nics_added_len)
            .field("hotplug_req_nics_removed_len", &nics_removed_len)
            .field("hotplug_req_volumes_added_len", &volumes_added_len)
            .field("hotplug_req_volumes_removed_len", &volumes_removed_len)
            .field("actual_cpu_cores", &self.actual_allocation.cpu_cores)
            .field("actual_memory_size", &self.actual_allocation.memory_size)
            .field("actual_nic_ids_len", &self.actual_allocation.nic_ids.len())
            .field(
                "actual_volume_ids_len",
                &self.actual_allocation.volume_ids.len(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests;
