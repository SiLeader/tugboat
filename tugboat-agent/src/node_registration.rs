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
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};
use tugboat_client::{Api, TugboatClient};
use tugboat_cni_operator::CniOperatorConfig;
use tugboat_resources::manifests::core::v1::{
    Node, NodeCniPluginStatus, NodeCondition, NodeOvercommitSpec, NodeResource, NodeSpec,
    NodeStatus,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, Time};

const PROC_CPUINFO_PATH: &str = "/proc/cpuinfo";
const PROC_MEMINFO_PATH: &str = "/proc/meminfo";
const CGROUP_V2_CPU_MAX_PATH: &str = "/sys/fs/cgroup/cpu.max";
const CGROUP_V2_MEMORY_MAX_PATH: &str = "/sys/fs/cgroup/memory.max";
const CGROUP_V1_CPU_QUOTA_PATH: &str = "/sys/fs/cgroup/cpu/cpu.cfs_quota_us";
const CGROUP_V1_CPU_PERIOD_PATH: &str = "/sys/fs/cgroup/cpu/cpu.cfs_period_us";
const CGROUP_V1_MEMORY_LIMIT_PATH: &str = "/sys/fs/cgroup/memory/memory.limit_in_bytes";
const CGROUP_V1_MEMORY_UNLIMITED_THRESHOLD: u64 = 1 << 60;
const DEFAULT_FLANNEL_SUBNET_FILE: &str = "/run/flannel/subnet.env";
const DEFAULT_FLANNEL_DATA_DIR: &str = "/run/flannel";
const REQUIRED_CNI_PLUGINS: [&str; 2] = ["bridge", "loopback"];
const OPTIONAL_CNI_PLUGINS: [&str; 2] = ["flannel", "portmap"];

#[derive(Debug, thiserror::Error)]
pub(crate) enum NodeRegistrationError {
    #[error("API access failed: {0}")]
    Client(#[from] tugboat_client::Error),
    #[error("Capacity detection failed: {0}")]
    Io(#[from] io::Error),
    #[error("Node resource '{0}' not found")]
    NodeMissing(String),
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

pub(crate) async fn publish_node_status(
    client: TugboatClient,
    node_name: &str,
    cni: &CniOperatorConfig,
) -> Result<(), NodeRegistrationError> {
    let api: Api<Node> = Api::all(client);
    let Some(mut node) = api.get(node_name).await? else {
        return Err(NodeRegistrationError::NodeMissing(node_name.to_string()));
    };

    node.status = Some(build_node_status(cni)?);
    api.replace_status(node_name, node).await?;
    debug!("Published CNI status for node '{node_name}'");
    Ok(())
}

pub(crate) async fn refresh_node_status_loop(
    client: TugboatClient,
    node_name: String,
    cni: CniOperatorConfig,
    interval: Duration,
) {
    info!(
        "Starting CNI capability reporter for node '{}' (interval={}s)",
        node_name,
        interval.as_secs()
    );

    loop {
        if let Err(err) = publish_node_status(client.clone(), &node_name, &cni).await {
            warn!(
                "Failed to publish CNI capability for node '{}': {}",
                node_name, err
            );
        }
        tokio::time::sleep(interval).await;
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
        status: None,
    }
}

fn build_node_status(cni: &CniOperatorConfig) -> Result<NodeStatus, io::Error> {
    build_node_status_with_flannel_paths(
        cni,
        Path::new(DEFAULT_FLANNEL_SUBNET_FILE),
        Path::new(DEFAULT_FLANNEL_DATA_DIR),
    )
}

fn build_node_status_with_flannel_paths(
    cni: &CniOperatorConfig,
    flannel_subnet_file: &Path,
    flannel_data_dir: &Path,
) -> Result<NodeStatus, io::Error> {
    let mut cni_plugins = REQUIRED_CNI_PLUGINS
        .iter()
        .map(|plugin| probe_plugin_binary(cni.bin_dir(), plugin))
        .collect::<Result<Vec<_>, _>>()?;
    cni_plugins.push(probe_flannel_plugin(
        cni.bin_dir(),
        flannel_subnet_file,
        flannel_data_dir,
    )?);
    cni_plugins.extend(
        OPTIONAL_CNI_PLUGINS[1..]
            .iter()
            .map(|plugin| probe_plugin_binary(cni.bin_dir(), plugin))
            .collect::<Result<Vec<_>, _>>()?,
    );

    let missing_required = cni_plugins
        .iter()
        .filter(|plugin| {
            REQUIRED_CNI_PLUGINS.contains(&plugin.name.as_str()) && !plugin.ready.unwrap_or(false)
        })
        .map(|plugin| plugin.name.clone())
        .collect::<Vec<_>>();

    let condition = NodeCondition {
        r#type: "CniReady".to_string(),
        status: if missing_required.is_empty() {
            "True".to_string()
        } else {
            "False".to_string()
        },
        message: if missing_required.is_empty() {
            "Required CNI plugins are available on this node.".to_string()
        } else {
            format!(
                "Missing required CNI plugins: {}",
                missing_required.join(", ")
            )
        },
        timestamp: Some(Time::now()),
    };

    Ok(NodeStatus {
        conditions: vec![condition],
        cni_plugins,
    })
}

fn probe_plugin_binary(bin_dir: &str, plugin: &str) -> Result<NodeCniPluginStatus, io::Error> {
    let binary = PathBuf::from(bin_dir).join(plugin);
    let ready = binary.try_exists()?;
    Ok(NodeCniPluginStatus {
        name: plugin.to_string(),
        ready: Some(ready),
        message: if ready {
            format!("Found plugin binary at '{}'.", binary.display())
        } else {
            format!("Missing plugin binary at '{}'.", binary.display())
        },
    })
}

fn probe_flannel_plugin(
    bin_dir: &str,
    subnet_file: &Path,
    data_dir: &Path,
) -> Result<NodeCniPluginStatus, io::Error> {
    let binary = PathBuf::from(bin_dir).join("flannel");
    let binary_ready = binary.try_exists()?;
    let subnet_ready = subnet_file.try_exists()?;
    let data_dir_ready = data_dir.try_exists()?;
    let ready = binary_ready && subnet_ready && data_dir_ready;

    let mut missing = Vec::new();
    if !binary_ready {
        missing.push(format!("plugin binary '{}'", binary.display()));
    }
    if !subnet_ready {
        missing.push(format!("subnet file '{}'", subnet_file.display()));
    }
    if !data_dir_ready {
        missing.push(format!("data dir '{}'", data_dir.display()));
    }

    Ok(NodeCniPluginStatus {
        name: "flannel".to_string(),
        ready: Some(ready),
        message: if ready {
            format!(
                "Found flannel plugin binary and runtime state ('{}', '{}').",
                subnet_file.display(),
                data_dir.display()
            )
        } else {
            format!("Missing flannel prerequisites: {}.", missing.join(", "))
        },
    })
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
        assert!(node.status.is_none());
    }

    #[test]
    fn build_node_status_reports_missing_required_plugins() {
        let base = test_dir("missing-required");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let config = test_cni_config(&bin);
        let subnet = base.join("subnet.env");
        let data_dir = base.join("flannel");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(&subnet, "FLANNEL_NETWORK=10.244.0.0/16").unwrap();

        let status = build_node_status_with_flannel_paths(&config, &subnet, &data_dir).unwrap();
        let condition = status.conditions.first().expect("condition should exist");

        assert_eq!(condition.r#type, "CniReady");
        assert_eq!(condition.status, "False");
        assert!(condition.message.contains("bridge"));
        assert!(condition.message.contains("loopback"));

        cleanup_test_dir(&base);
    }

    #[test]
    fn build_node_status_reports_flannel_prerequisites() {
        let base = test_dir("flannel-prerequisites");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for plugin in ["bridge", "loopback", "flannel", "portmap"] {
            std::fs::write(bin.join(plugin), "").unwrap();
        }
        let subnet = base.join("subnet.env");
        let data_dir = base.join("flannel");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(&subnet, "FLANNEL_NETWORK=10.244.0.0/16").unwrap();

        let config = test_cni_config(&bin);
        let status = build_node_status_with_flannel_paths(&config, &subnet, &data_dir).unwrap();

        let flannel = status
            .cni_plugins
            .iter()
            .find(|plugin| plugin.name == "flannel")
            .expect("flannel plugin should exist");
        let ready = flannel.ready.expect("flannel readiness should be present");

        assert!(ready);
        assert!(flannel.message.contains("runtime state"));
        assert_eq!(status.conditions[0].status, "True");

        cleanup_test_dir(&base);
    }

    fn test_cni_config(bin_dir: &Path) -> CniOperatorConfig {
        toml::from_str(&format!(
            r#"[location]
bin = "{}"
config = "{}"
netns = "{}"
"#,
            bin_dir.display(),
            bin_dir.join("config").display(),
            bin_dir.join("netns").display()
        ))
        .unwrap()
    }

    fn test_dir(name: &str) -> PathBuf {
        let unique = format!(
            "tugboat-node-registration-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn cleanup_test_dir(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }
}
