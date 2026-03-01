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

use std::io;
use tracing::info;
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{Node, NodeOvercommitSpec, NodeResource, NodeSpec};
use tugboat_resources::manifests::meta::v1::ObjectMeta;

const PROC_CPUINFO_PATH: &str = "/proc/cpuinfo";
const PROC_MEMINFO_PATH: &str = "/proc/meminfo";
const CGROUP_V2_CPU_MAX_PATH: &str = "/sys/fs/cgroup/cpu.max";
const CGROUP_V2_MEMORY_MAX_PATH: &str = "/sys/fs/cgroup/memory.max";
const CGROUP_V1_CPU_QUOTA_PATH: &str = "/sys/fs/cgroup/cpu/cpu.cfs_quota_us";
const CGROUP_V1_CPU_PERIOD_PATH: &str = "/sys/fs/cgroup/cpu/cpu.cfs_period_us";
const CGROUP_V1_MEMORY_LIMIT_PATH: &str = "/sys/fs/cgroup/memory/memory.limit_in_bytes";
const CGROUP_V1_MEMORY_UNLIMITED_THRESHOLD: u64 = 1 << 60;

#[derive(Debug, thiserror::Error)]
pub(crate) enum NodeRegistrationError {
    #[error("API access failed: {0}")]
    Client(#[from] tugboat_client::Error),
    #[error("Capacity detection failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NodeCapacity {
    cpu: u64,
    memory: u64,
}

pub(crate) async fn ensure_node_exists(
    client: TugboatClient,
    node_name: String,
) -> Result<(), NodeRegistrationError> {
    let api: Api<Node> = Api::all(client.clone());

    if api.get(&node_name).await?.is_some() {
        info!("Node resource '{node_name}' already exists. Skipping registration.");
        return Ok(());
    }

    let capacity = detect_node_capacity()?;
    let node = build_node(node_name.clone(), capacity);

    match api.create(node).await {
        Ok(_) => {
            info!(
                "Node resource '{node_name}' created (cpu={}, memory={})",
                capacity.cpu, capacity.memory
            );
            Ok(())
        }
        Err(err) => {
            if api.get(&node_name).await?.is_some() {
                info!(
                    "Node resource '{node_name}' was created concurrently. Skipping registration."
                );
                Ok(())
            } else {
                Err(NodeRegistrationError::Client(err))
            }
        }
    }
}

fn build_node(node_name: String, capacity: NodeCapacity) -> Node {
    Node {
        type_meta: None,
        object_meta: Some(ObjectMeta {
            name: Some(node_name),
            ..Default::default()
        }),
        spec: Some(NodeSpec {
            overcommit: Some(NodeOvercommitSpec {
                cpu_ratio: "1".to_string(),
                memory_ratio: "1".to_string(),
            }),
            ips: Vec::new(),
            resource: Some(NodeResource {
                cpu: capacity.cpu,
                memory: capacity.memory,
            }),
            taints: Vec::new(),
        }),
    }
}

fn detect_node_capacity() -> Result<NodeCapacity, io::Error> {
    let physical_cpu = detect_physical_cpu()?;
    let physical_memory = detect_physical_memory()?;

    let cpu = detect_cgroup_cpu_limit().unwrap_or(physical_cpu).max(1);
    let memory = detect_cgroup_memory_limit().unwrap_or(physical_memory);

    Ok(NodeCapacity { cpu, memory })
}

fn detect_cgroup_cpu_limit() -> Option<u64> {
    std::fs::read_to_string(CGROUP_V2_CPU_MAX_PATH)
        .ok()
        .as_deref()
        .and_then(parse_cgroup_v2_cpu_max)
        .or_else(|| {
            let quota = std::fs::read_to_string(CGROUP_V1_CPU_QUOTA_PATH).ok()?;
            let period = std::fs::read_to_string(CGROUP_V1_CPU_PERIOD_PATH).ok()?;
            parse_cgroup_v1_cpu_limit(&quota, &period)
        })
}

fn detect_cgroup_memory_limit() -> Option<u64> {
    std::fs::read_to_string(CGROUP_V2_MEMORY_MAX_PATH)
        .ok()
        .as_deref()
        .and_then(parse_cgroup_v2_memory_max)
        .or_else(|| {
            let limit = std::fs::read_to_string(CGROUP_V1_MEMORY_LIMIT_PATH).ok()?;
            parse_cgroup_v1_memory_limit(&limit)
        })
}

fn detect_physical_cpu() -> Result<u64, io::Error> {
    let cpuinfo = std::fs::read_to_string(PROC_CPUINFO_PATH)?;
    let cores = cpuinfo
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            line.starts_with("processor\t:") || line.starts_with("processor :")
        })
        .count() as u64;

    if cores > 0 {
        return Ok(cores);
    }

    std::thread::available_parallelism()
        .map(|v| v.get() as u64)
        .map_err(|e| io::Error::other(format!("failed to detect CPU cores: {e}")))
}

fn detect_physical_memory() -> Result<u64, io::Error> {
    let meminfo = std::fs::read_to_string(PROC_MEMINFO_PATH)?;
    parse_mem_total_bytes(&meminfo)
        .ok_or_else(|| io::Error::other("failed to parse MemTotal from /proc/meminfo"))
}

fn parse_cgroup_v2_cpu_max(value: &str) -> Option<u64> {
    let mut values = value.split_whitespace();
    let quota = values.next()?;
    let period = values.next()?;

    if quota == "max" {
        return None;
    }

    let quota = quota.parse::<u64>().ok()?;
    let period = period.parse::<u64>().ok()?;

    parse_cpu_limit(quota, period)
}

fn parse_cgroup_v1_cpu_limit(quota: &str, period: &str) -> Option<u64> {
    let quota = quota.trim().parse::<i64>().ok()?;
    if quota < 0 {
        return None;
    }

    let period = period.trim().parse::<u64>().ok()?;
    parse_cpu_limit(quota as u64, period)
}

fn parse_cpu_limit(quota: u64, period: u64) -> Option<u64> {
    if quota == 0 || period == 0 {
        return None;
    }

    Some((quota / period).max(1))
}

fn parse_cgroup_v2_memory_max(value: &str) -> Option<u64> {
    let value = value.trim();
    if value == "max" {
        return None;
    }

    parse_memory_limit(value)
}

fn parse_cgroup_v1_memory_limit(value: &str) -> Option<u64> {
    let limit = parse_memory_limit(value.trim())?;
    if limit >= CGROUP_V1_MEMORY_UNLIMITED_THRESHOLD {
        return None;
    }
    Some(limit)
}

fn parse_memory_limit(value: &str) -> Option<u64> {
    let parsed = value.parse::<u64>().ok()?;
    if parsed == 0 {
        return None;
    }
    Some(parsed)
}

fn parse_mem_total_bytes(meminfo: &str) -> Option<u64> {
    let total_kib = meminfo.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("MemTotal:")?;
        let mut values = rest.split_whitespace();
        let value = values.next()?;
        let unit = values.next()?;
        if unit != "kB" {
            return None;
        }
        value.parse::<u64>().ok()
    })?;

    Some(total_kib.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_cgroup_v2_cpu_limit() {
        assert_eq!(parse_cgroup_v2_cpu_max("200000 100000"), Some(2));
        assert_eq!(parse_cgroup_v2_cpu_max("50000 100000"), Some(1));
        assert_eq!(parse_cgroup_v2_cpu_max("max 100000"), None);
    }

    #[test]
    fn can_parse_cgroup_v1_cpu_limit() {
        assert_eq!(parse_cgroup_v1_cpu_limit("200000", "100000"), Some(2));
        assert_eq!(parse_cgroup_v1_cpu_limit("50000", "100000"), Some(1));
        assert_eq!(parse_cgroup_v1_cpu_limit("-1", "100000"), None);
    }

    #[test]
    fn can_parse_cgroup_v2_memory_limit() {
        assert_eq!(
            parse_cgroup_v2_memory_max("1073741824"),
            Some(1_073_741_824)
        );
        assert_eq!(parse_cgroup_v2_memory_max("max"), None);
    }

    #[test]
    fn can_parse_cgroup_v1_memory_limit() {
        assert_eq!(parse_cgroup_v1_memory_limit("9223372036854771712"), None);
        assert_eq!(
            parse_cgroup_v1_memory_limit("2147483648"),
            Some(2_147_483_648)
        );
    }

    #[test]
    fn can_parse_mem_total_bytes() {
        let input = "MemTotal:       16384256 kB\nMemFree:         1024000 kB\n";
        assert_eq!(parse_mem_total_bytes(input), Some(16_777_478_144));
    }

    #[test]
    fn build_node_sets_overcommit_to_one() {
        let node = build_node(
            "node1".to_string(),
            NodeCapacity {
                cpu: 4,
                memory: 8_589_934_592,
            },
        );

        let spec = node.spec.expect("spec should exist");
        let overcommit = spec.overcommit.expect("overcommit should exist");
        let resource = spec.resource.expect("resource should exist");

        assert_eq!(overcommit.cpu_ratio, "1");
        assert_eq!(overcommit.memory_ratio, "1");
        assert_eq!(resource.cpu, 4);
        assert_eq!(resource.memory, 8_589_934_592);
    }
}
