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

use sha2::Digest;
use tugboat_cni_operator::{
    CniConfContent, CniConfHeader, CniIpam, CniIpamRoute, CniNetConfList, CniOperator,
};
use tugboat_resources::manifests::core::v1::NetworkClassSpec;
use tugboat_vm_runtime_interface::run::VmNetworkConfig;

#[derive(Debug, Clone)]
pub(crate) struct CniWrapper {
    operator: CniOperator,
}

#[derive(Debug, Clone)]
pub(crate) struct NetworkClassInfo {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: NetworkClassSpec,
}

#[derive(Debug, Clone)]
pub(crate) struct PlannedNetworkConfig {
    pub bridge: String,
    pub info: NetworkClassInfo,
    pub vm: VmNetworkConfig,
}

impl CniWrapper {
    pub(crate) fn new(operator: CniOperator) -> Self {
        Self { operator }
    }

    pub(crate) fn create_network_configs(
        &self,
        ship_id: &str,
        network_classes: Vec<NetworkClassInfo>,
    ) -> Vec<PlannedNetworkConfig> {
        let mut planned = Vec::new();
        for (idx, network_class) in network_classes.into_iter().enumerate() {
            let iface_name = format!("eth{}", idx);
            let plan = Self::plan_single(ship_id, iface_name, network_class);
            planned.push(plan);
        }
        planned
    }

    fn plan_single(
        ship_id: &str,
        iface_name: String,
        network_class: NetworkClassInfo,
    ) -> PlannedNetworkConfig {
        let bridge = create_bridge_name(&network_class.namespace, &network_class.name);
        let mac = mac_address(&network_class.namespace, &network_class.name, ship_id);
        PlannedNetworkConfig {
            bridge,
            info: network_class,
            vm: VmNetworkConfig {
                iface_name,
                mac_address: mac,
            },
        }
    }

    pub(crate) async fn add(
        &self,
        ship_id: &str,
        config: Vec<PlannedNetworkConfig>,
    ) -> Result<Vec<VmNetworkConfig>, tugboat_cni_operator::Error> {
        self.add_loopback(ship_id).await?;
        let mut applied = Vec::new();
        for c in config {
            let net = self.add_single(ship_id, c).await?;
            applied.push(net);
        }
        Ok(applied)
    }

    async fn add_loopback(&self, ship_id: &str) -> Result<(), tugboat_cni_operator::Error> {
        let conf = CniNetConfList {
            header: CniConfHeader {
                cni_version: "1.0.0".to_string(),
                name: "loopback".to_string(),
            },
            plugins: vec![CniConfContent::Loopback],
        };
        self.operator.add(ship_id, "lo", "loopback", conf).await?;
        Ok(())
    }

    async fn add_single(
        &self,
        ship_id: &str,
        config: PlannedNetworkConfig,
    ) -> Result<VmNetworkConfig, tugboat_cni_operator::Error> {
        let conf = CniNetConfList {
            header: CniConfHeader {
                cni_version: "1.0.0".to_string(),
                name: config.info.name,
            },
            plugins: vec![CniConfContent::Bridge {
                bridge: config.bridge,
                is_gateway: config.info.spec.cluster_network.unwrap_or(true),
                ip_masquerade: config.info.spec.internet_access.unwrap_or(false),
                ipam: CniIpam {
                    cni_type: "host-local".to_string(),
                    subnet: config.info.spec.subnet,
                    routes: config
                        .info
                        .spec
                        .routes
                        .into_iter()
                        .map(|route| CniIpamRoute {
                            destination: route.destination,
                        })
                        .collect(),
                },
            }],
        };
        self.operator
            .add(ship_id, &config.vm.iface_name, "bridge", conf)
            .await?;
        Ok(config.vm)
    }
}

fn mac_address(namespace: &Option<String>, name: &str, ship_id: &str) -> String {
    let ident = match &namespace {
        Some(ns) => format!("NetworkClass/{ns}/{name}/{ship_id}"),
        None => format!("ClusterNetworkClass/{name}/{ship_id}"),
    };
    let digest = &sha2::Sha256::digest(ident.as_bytes());
    format!(
        "52:54:00:{digest:02x}:{digest:02x}:{digest:02x}",
        digest = digest
    )
}

fn create_bridge_name(namespace: &Option<String>, name: &str) -> String {
    let (prefix, ident) = match &namespace {
        Some(ns) => ("ns", format!("NetworkClass/{ns}/{}", name)),
        None => ("cl", format!("ClusterNetworkClass/{}", name)),
    };
    let digest = format!("{:x}", sha2::Sha256::digest(ident.as_bytes()));
    format!("br-{}-{}", prefix, &digest[..8])
}

#[cfg(test)]
mod tests {
    use crate::cni::create_bridge_name;

    #[test]
    fn test_create_bridge_name() {
        let actual = create_bridge_name(&Some("ns".to_string()), "test");
        assert!(actual.len() < 15); // Linux's interface name length is 15 characters or fewer.
        assert_eq!(actual, "br-ns-8ead8051");

        let actual = create_bridge_name(&None, "test");
        assert!(actual.len() < 15); // Linux's interface name length is 15 characters or fewer.
        assert_eq!(actual, "br-cl-d5b95190");
    }
}
