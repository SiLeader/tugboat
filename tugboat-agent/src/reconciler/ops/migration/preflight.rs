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

use crate::cni::NetworkClassInfo;
use crate::csi::READ_WRITE_MANY;
use crate::reconciler::volume::VolumeInfo;
use std::collections::BTreeSet;
use tugboat_resources::NODE_ARCH_LABEL_KEY;
use tugboat_resources::manifests::core::v1::{Node, NodeCniPluginStatus, Ship, ShipClass};

pub(super) fn validate_target_node_readiness(target_node: &Node) -> Option<String> {
    let Some(meta) = target_node.object_meta.as_ref() else {
        return Some("target node is missing metadata".to_string());
    };
    let node_name = meta.name.as_deref().unwrap_or("<unknown>");
    let Some(spec) = target_node.spec.as_ref() else {
        return Some(format!("target node '{node_name}' is missing spec"));
    };

    if !spec.ips.iter().any(|ip| {
        ip.parse::<std::net::IpAddr>()
            .map(|addr| !addr.is_loopback())
            .unwrap_or(false)
    }) {
        return Some(format!(
            "target node '{node_name}' does not advertise a reachable non-loopback IP"
        ));
    }

    let Some(status) = target_node.status.as_ref() else {
        return Some(format!("target node '{node_name}' has no published status"));
    };
    let Some(condition) = status
        .conditions
        .iter()
        .find(|condition| condition.r#type == "CniReady")
    else {
        return Some(format!(
            "target node '{node_name}' does not publish a CniReady condition"
        ));
    };

    if condition.status == "True" {
        None
    } else if condition.message.is_empty() {
        Some(format!("target node '{node_name}' is not CNI-ready"))
    } else {
        Some(format!(
            "target node '{node_name}' is not CNI-ready: {}",
            condition.message
        ))
    }
}

pub(super) fn validate_target_architecture(
    ship_class: &ShipClass,
    target_node: &Node,
) -> Option<String> {
    let requested = ship_class
        .spec
        .as_ref()
        .and_then(|spec| spec.cpu.as_ref())
        .map(|cpu| normalize_architecture(&cpu.architecture))
        .filter(|arch| !arch.is_empty())?;
    let meta = target_node.object_meta.as_ref()?;
    let node_name = meta.name.as_deref().unwrap_or("<unknown>");
    let Some(actual) = meta
        .labels
        .get(NODE_ARCH_LABEL_KEY)
        .map(|arch| normalize_architecture(arch))
    else {
        return Some(format!(
            "target node '{node_name}' does not advertise '{}' label",
            NODE_ARCH_LABEL_KEY
        ));
    };

    if requested == actual {
        None
    } else {
        Some(format!(
            "target node '{node_name}' architecture '{actual}' is incompatible with ship class architecture '{requested}'"
        ))
    }
}

pub(super) fn validate_target_network_capability(
    target_node: &Node,
    network_classes: &[NetworkClassInfo],
) -> Option<String> {
    let statuses = target_node
        .status
        .as_ref()
        .map(|status| status.cni_plugins.as_slice())
        .unwrap_or(&[]);
    let mut required_plugins = BTreeSet::from(["loopback".to_string()]);

    for network_class in network_classes {
        let plugin = normalized_plugin(&network_class.spec);
        if plugin.eq_ignore_ascii_case("bridge") {
            required_plugins.insert("bridge".to_string());
        } else if plugin.eq_ignore_ascii_case("flannel") {
            required_plugins.insert("bridge".to_string());
            required_plugins.insert("flannel".to_string());
            if network_class
                .spec
                .flannel
                .as_ref()
                .and_then(|flannel| flannel.port_mappings)
                .unwrap_or(false)
            {
                required_plugins.insert("portmap".to_string());
            }
        } else {
            return Some(format!(
                "network class '{}' requires unsupported cniPlugin '{}'",
                network_class.name, plugin
            ));
        }
    }

    for plugin in required_plugins {
        if let Some(reason) = require_plugin_ready(statuses, &plugin) {
            return Some(reason);
        }
    }

    None
}

pub(super) fn validate_storage_eligibility(volumes: &[VolumeInfo]) -> Option<String> {
    for volume in volumes {
        let Some(volume) = volume.persistent_volume_claim() else {
            continue;
        };

        let claim_supports_rwx = volume
            .claim
            .access_modes
            .iter()
            .any(|mode| mode == READ_WRITE_MANY);
        let pv_supports_rwx = volume
            .volume
            .access_modes
            .iter()
            .any(|mode| mode == READ_WRITE_MANY);

        if !claim_supports_rwx || !pv_supports_rwx {
            return Some(format!(
                "persistent volume claim '{}' must use shared storage with '{}' access on both the claim and persistent volume for live migration",
                volume.claim_name, READ_WRITE_MANY
            ));
        }
    }

    None
}

pub(super) fn validate_target_resource_capacity(
    target_node: &Node,
    ships: &[Ship],
    ship_classes: &[ShipClass],
    requested_ship_class: &ShipClass,
) -> Option<String> {
    let node_name = target_node
        .object_meta
        .as_ref()
        .and_then(|meta| meta.name.as_deref())
        .unwrap_or("<unknown>");

    let (alloc_cpu, alloc_memory) = node_allocatable(target_node);
    let (used_cpu, used_memory) = node_resource_usage(ships, ship_classes, node_name);
    let (req_cpu, req_memory) = ship_class_requested_resources(requested_ship_class);

    let avail_cpu = alloc_cpu.saturating_sub(used_cpu);
    if req_cpu > avail_cpu {
        return Some(format!(
            "target node '{node_name}' has insufficient CPU for live migration: requested={req_cpu}, available={avail_cpu}"
        ));
    }

    let avail_memory = alloc_memory.saturating_sub(used_memory);
    if req_memory > avail_memory {
        return Some(format!(
            "target node '{node_name}' has insufficient memory for live migration: requested={req_memory}, available={avail_memory}"
        ));
    }

    None
}

fn node_allocatable(node: &Node) -> (u64, u64) {
    let spec = node.spec.as_ref();
    let resource = spec.and_then(|s| s.resource.as_ref());
    let overcommit = spec.and_then(|s| s.overcommit.as_ref());

    let base_cpu = resource.map(|r| r.cpu).unwrap_or(0);
    let base_memory = resource.map(|r| r.memory).unwrap_or(0);

    let cpu_ratio: f64 = overcommit
        .and_then(|o| o.cpu_ratio.parse().ok())
        .unwrap_or(1.0);
    let memory_ratio: f64 = overcommit
        .and_then(|o| o.memory_ratio.parse().ok())
        .unwrap_or(1.0);

    let alloc_cpu = (base_cpu as f64 * cpu_ratio) as u64;
    let alloc_memory = (base_memory as f64 * memory_ratio) as u64;

    (alloc_cpu, alloc_memory)
}

fn node_resource_usage(ships: &[Ship], ship_classes: &[ShipClass], node_name: &str) -> (u64, u64) {
    let mut cpu_used: u64 = 0;
    let mut memory_used: u64 = 0;

    for ship in ships {
        let assigned_node = ship
            .spec
            .as_ref()
            .and_then(|spec| spec.node_name.as_deref());
        if assigned_node != Some(node_name) {
            continue;
        }

        let class_name = ship
            .spec
            .as_ref()
            .map(|spec| spec.ship_class.as_str())
            .unwrap_or("");
        let Some(ship_class) = find_ship_class(ship_classes, class_name) else {
            continue;
        };
        let (cpu, memory) = ship_class_requested_resources(ship_class);
        cpu_used = cpu_used.saturating_add(cpu);
        memory_used = memory_used.saturating_add(memory);
    }

    (cpu_used, memory_used)
}

fn find_ship_class<'a>(ship_classes: &'a [ShipClass], name: &str) -> Option<&'a ShipClass> {
    ship_classes.iter().find(|ship_class| {
        ship_class
            .object_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            == Some(name)
    })
}

fn ship_class_requested_resources(ship_class: &ShipClass) -> (u64, u64) {
    let spec = ship_class.spec.as_ref();
    let cpu = spec
        .and_then(|spec| spec.cpu.as_ref())
        .map(|cpu| cpu.cores)
        .unwrap_or(0);
    let memory = spec
        .and_then(|spec| spec.memory.as_ref())
        .map(|memory| parse_memory_size(&memory.size))
        .unwrap_or(0);
    (cpu, memory)
}

fn parse_memory_size(s: &str) -> u64 {
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

    let Ok(num) = num_str.parse::<f64>() else {
        return 0;
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

pub(super) fn normalize_architecture(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => "amd64".to_string(),
        "aarch64" | "arm64" => "arm64".to_string(),
        other => other.to_string(),
    }
}

fn normalized_plugin(spec: &tugboat_resources::manifests::core::v1::NetworkClassSpec) -> &str {
    let plugin = spec.cni_plugin.trim();
    if plugin.is_empty() { "bridge" } else { plugin }
}

fn require_plugin_ready(statuses: &[NodeCniPluginStatus], plugin: &str) -> Option<String> {
    let Some(status) = statuses.iter().find(|status| status.name == plugin) else {
        return Some(format!(
            "target node does not advertise required CNI plugin '{}'",
            plugin
        ));
    };

    if status.ready.unwrap_or(false) {
        None
    } else if status.message.is_empty() {
        Some(format!("required CNI plugin '{}' is not ready", plugin))
    } else {
        Some(format!(
            "required CNI plugin '{}' is not ready: {}",
            plugin, status.message
        ))
    }
}
