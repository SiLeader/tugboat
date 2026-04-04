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

use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, NetworkClass, PersistentVolume, PersistentVolumeClaim, Ship, ShipClass,
};

/// Context shared across plugin invocations for a single scheduling cycle.
pub struct SchedulingContext {
    /// The Ship being scheduled.
    pub ship: Ship,
    /// The ShipClass referenced by the Ship.
    pub ship_class: ShipClass,
    /// All cluster-scoped network classes.
    pub all_cluster_network_classes: Vec<ClusterNetworkClass>,
    /// All namespaced network classes.
    pub all_network_classes: Vec<NetworkClass>,
    /// All Ships currently in the cluster (for resource usage calculation).
    pub all_ships: Vec<Ship>,
    /// All ShipClasses (for resolving resource requirements of scheduled ships).
    pub all_ship_classes: Vec<ShipClass>,
    /// All PersistentVolumeClaims in the cluster (for storage-fit checks).
    pub all_persistent_volume_claims: Vec<PersistentVolumeClaim>,
    /// All PersistentVolumes in the cluster (for storage-fit checks).
    pub all_persistent_volumes: Vec<PersistentVolume>,
}

impl SchedulingContext {
    /// Calculate the total CPU and memory used by Ships assigned to a given node.
    /// Returns (cpu_used, memory_used_bytes).
    pub fn node_resource_usage(&self, node_name: &str) -> (u64, u64) {
        let mut cpu_used: u64 = 0;
        let mut memory_used: u64 = 0;

        for ship in &self.all_ships {
            let assigned_node = ship.spec.as_ref().and_then(|s| s.node_name.as_deref());
            if assigned_node != Some(node_name) {
                continue;
            }

            let class_name = ship
                .spec
                .as_ref()
                .map(|s| s.ship_class.as_str())
                .unwrap_or("");
            if let Some(sc) = self.find_ship_class(class_name)
                && let Some(spec) = sc.spec.as_ref()
            {
                if let Some(cpu) = spec.cpu.as_ref() {
                    cpu_used = cpu_used.saturating_add(cpu.cores);
                }
                if let Some(mem) = spec.memory.as_ref() {
                    memory_used = memory_used.saturating_add(parse_memory_size(&mem.size));
                }
            }
        }

        (cpu_used, memory_used)
    }

    fn find_ship_class(&self, name: &str) -> Option<&ShipClass> {
        self.all_ship_classes
            .iter()
            .find(|sc| sc.object_meta.as_ref().and_then(|m| m.name.as_deref()) == Some(name))
    }

    /// Return PVs bound to PVCs referenced by the Ship being scheduled.
    /// Only PVC-backed volumes are included; ConfigMap/Secret volumes are skipped.
    pub fn ship_bound_persistent_volumes(&self) -> Vec<&PersistentVolume> {
        let namespace = self.ship_namespace();
        let Some(spec) = self.ship.spec.as_ref() else {
            return Vec::new();
        };

        let mut result = Vec::new();
        for volume in &spec.volumes {
            let Some(pvc_source) = volume.persistent_volume_claim.as_ref() else {
                continue;
            };
            let claim_name = &pvc_source.claim_name;
            let Some(pvc) = self.all_persistent_volume_claims.iter().find(|pvc| {
                let meta = pvc.object_meta.as_ref();
                meta.and_then(|m| m.name.as_deref()) == Some(claim_name.as_str())
                    && meta.and_then(|m| m.namespace.as_deref()) == Some(namespace)
            }) else {
                continue;
            };
            let pv_name = pvc
                .spec
                .as_ref()
                .and_then(|s| s.volume_name.as_deref())
                .unwrap_or("");
            if pv_name.is_empty() {
                continue;
            }
            if let Some(pv) = self
                .all_persistent_volumes
                .iter()
                .find(|pv| pv.object_meta.as_ref().and_then(|m| m.name.as_deref()) == Some(pv_name))
            {
                result.push(pv);
            }
        }
        result
    }

    pub fn ship_namespace(&self) -> &str {
        self.ship
            .object_meta
            .as_ref()
            .and_then(|meta| meta.namespace.as_deref())
            .unwrap_or("default")
    }

    pub fn find_cluster_network_class(&self, name: &str) -> Option<&ClusterNetworkClass> {
        self.all_cluster_network_classes
            .iter()
            .find(|network_class| {
                network_class
                    .object_meta
                    .as_ref()
                    .and_then(|meta| meta.name.as_deref())
                    == Some(name)
            })
    }

    pub fn find_network_class(&self, namespace: &str, name: &str) -> Option<&NetworkClass> {
        self.all_network_classes.iter().find(|network_class| {
            let meta = network_class.object_meta.as_ref();
            meta.and_then(|item| item.name.as_deref()) == Some(name)
                && meta.and_then(|item| item.namespace.as_deref()) == Some(namespace)
        })
    }

    /// Get CPU and memory requested by the Ship being scheduled.
    /// Returns (cpu_cores, memory_bytes).
    pub fn requested_resources(&self) -> (u64, u64) {
        let spec = self.ship_class.spec.as_ref();
        let cpu = spec
            .and_then(|s| s.cpu.as_ref())
            .map(|c| c.cores)
            .unwrap_or(0);
        let memory = spec
            .and_then(|s| s.memory.as_ref())
            .map(|m| parse_memory_size(&m.size))
            .unwrap_or(0);
        (cpu, memory)
    }
}

/// Parse a memory size string (e.g., "1Gi", "512Mi", "2G", "1024M", "1073741824") into bytes.
pub fn parse_memory_size(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    if let Ok(bytes) = s.parse::<u64>() {
        return bytes;
    }

    let (num_str, suffix) = if let Some(n) = s.strip_suffix("Gi") {
        (n, "Gi")
    } else if let Some(n) = s.strip_suffix("Mi") {
        (n, "Mi")
    } else if let Some(n) = s.strip_suffix("Ki") {
        (n, "Ki")
    } else if let Some(n) = s.strip_suffix("Ti") {
        (n, "Ti")
    } else if let Some(n) = s.strip_suffix('G') {
        (n, "G")
    } else if let Some(n) = s.strip_suffix('M') {
        (n, "M")
    } else if let Some(n) = s.strip_suffix('K') {
        (n, "K")
    } else if let Some(n) = s.strip_suffix('T') {
        (n, "T")
    } else {
        return 0;
    };

    let num: f64 = match num_str.parse() {
        Ok(n) => n,
        Err(_) => return 0,
    };

    let multiplier: u64 = match suffix {
        "Ki" => 1024,
        "Mi" => 1024 * 1024,
        "Gi" => 1024 * 1024 * 1024,
        "Ti" => 1024 * 1024 * 1024 * 1024,
        "K" => 1000,
        "M" => 1000 * 1000,
        "G" => 1000 * 1000 * 1000,
        "T" => 1000 * 1000 * 1000 * 1000,
        _ => return 0,
    };

    let result = num * multiplier as f64;
    if result >= u64::MAX as f64 {
        return u64::MAX;
    }
    result as u64
}

pub enum FilterResult {
    Accept,
    Reject(String),
}

pub enum ScoreResult {
    Score(i64),
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_size() {
        assert_eq!(parse_memory_size("1Gi"), 1024 * 1024 * 1024);
        assert_eq!(parse_memory_size("512Mi"), 512 * 1024 * 1024);
        assert_eq!(parse_memory_size("1G"), 1_000_000_000);
        assert_eq!(parse_memory_size("1024"), 1024);
        assert_eq!(parse_memory_size(""), 0);
        assert_eq!(parse_memory_size("2Ti"), 2 * 1024 * 1024 * 1024 * 1024);
    }
}
