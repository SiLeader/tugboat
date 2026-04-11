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
use tracing::{debug, info, warn};
use tugboat_cni_operator::{
    CniConfContent, CniConfHeader, CniFlannelDelegate, CniIpam, CniIpamRoute, CniNetConfList,
    CniPortmapCapabilities, TugboatCniOperator,
};
use tugboat_resources::manifests::core::v1::NetworkClassSpec;
use tugboat_vm_runtime_interface::run::VmNetworkConfig;

const CNI_VERSION: &str = "1.0.0";

#[derive(Debug, Clone)]
pub(crate) struct CniWrapper {
    operator: TugboatCniOperator,
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
    pub(crate) fn new(operator: TugboatCniOperator) -> Self {
        Self { operator }
    }

    pub(crate) fn create_network_configs(
        &self,
        ship_id: &str,
        network_classes: Vec<NetworkClassInfo>,
    ) -> Vec<PlannedNetworkConfig> {
        self.create_network_configs_from_index(ship_id, 0, network_classes)
    }

    pub(crate) fn create_network_configs_from_index(
        &self,
        ship_id: &str,
        start_index: usize,
        network_classes: Vec<NetworkClassInfo>,
    ) -> Vec<PlannedNetworkConfig> {
        debug!("Create network configuration plans for '{ship_id}'");
        let mut planned = Vec::new();
        for (offset, network_class) in network_classes.into_iter().enumerate() {
            let iface_name = format!("eth{}", start_index + offset);
            let plan = Self::plan_single(ship_id, iface_name, network_class);
            planned.push(plan);
        }
        debug!("Network configuration plans: {planned:?}");
        info!(
            "{} network configuration plans for '{ship_id}' was created.",
            planned.len()
        );
        planned
    }

    fn plan_single(
        ship_id: &str,
        iface_name: String,
        network_class: NetworkClassInfo,
    ) -> PlannedNetworkConfig {
        debug!(
            "Create network configuration plan: ship id = '{ship_id}' interface = '{iface_name}'"
        );
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

    pub(crate) async fn del(
        &self,
        ship_id: &str,
        config: Vec<PlannedNetworkConfig>,
    ) -> Result<(), tugboat_cni_operator::Error> {
        let mut errors = Vec::new();
        for c in config.into_iter().rev() {
            let iface_name = c.vm.iface_name.clone();
            if let Err(err) = self.del_single(ship_id, c).await {
                warn!(
                    "Failed to delete CNI config for ship '{}' iface '{}': {}",
                    ship_id, iface_name, err
                );
                errors.push(format!("{iface_name}: {err}"));
            }
        }

        if let Err(err) = self.del_loopback(ship_id).await {
            warn!(
                "Failed to delete loopback CNI config for ship '{}': {}",
                ship_id, err
            );
            errors.push(format!("lo: {err}"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(tugboat_cni_operator::Error::InvalidConfiguration(format!(
                "CNI delete completed with failures: {}",
                errors.join("; ")
            )))
        }
    }

    async fn add_loopback(&self, ship_id: &str) -> Result<(), tugboat_cni_operator::Error> {
        self.operator
            .add(ship_id, "lo", Self::loopback_conf())
            .await?;
        Ok(())
    }

    async fn del_loopback(&self, ship_id: &str) -> Result<(), tugboat_cni_operator::Error> {
        self.operator
            .del(ship_id, "lo", Self::loopback_conf())
            .await?;
        Ok(())
    }

    pub(crate) async fn add_single(
        &self,
        ship_id: &str,
        config: PlannedNetworkConfig,
    ) -> Result<VmNetworkConfig, tugboat_cni_operator::Error> {
        let conf = Self::network_conf(&config)?;
        self.operator
            .add(ship_id, &config.vm.iface_name, conf)
            .await?;
        Ok(config.vm)
    }

    pub(crate) async fn del_single(
        &self,
        ship_id: &str,
        config: PlannedNetworkConfig,
    ) -> Result<(), tugboat_cni_operator::Error> {
        let conf = Self::network_conf(&config)?;
        self.operator
            .del(ship_id, &config.vm.iface_name, conf)
            .await?;
        Ok(())
    }

    fn loopback_conf() -> CniNetConfList {
        CniNetConfList {
            header: CniConfHeader {
                cni_version: CNI_VERSION.to_string(),
                name: "loopback".to_string(),
            },
            plugins: vec![CniConfContent::Loopback],
        }
    }

    fn network_conf(
        config: &PlannedNetworkConfig,
    ) -> Result<CniNetConfList, tugboat_cni_operator::Error> {
        let plugin = config.info.spec.cni_plugin.trim();
        if plugin.is_empty() || plugin.eq_ignore_ascii_case("bridge") {
            Ok(Self::bridge_conf(config))
        } else if plugin.eq_ignore_ascii_case("flannel") {
            Ok(Self::flannel_conf(config))
        } else {
            Err(tugboat_cni_operator::Error::InvalidConfiguration(format!(
                "NetworkClass '{}' requests unsupported cniPlugin '{}'",
                network_class_identifier(&config.info),
                plugin
            )))
        }
    }

    fn bridge_conf(config: &PlannedNetworkConfig) -> CniNetConfList {
        CniNetConfList {
            header: CniConfHeader {
                cni_version: CNI_VERSION.to_string(),
                name: config.info.name.clone(),
            },
            plugins: vec![CniConfContent::Bridge {
                bridge: config.bridge.clone(),
                is_gateway: config.info.spec.cluster_network.unwrap_or(true),
                ip_masquerade: config.info.spec.internet_access.unwrap_or(false),
                ipam: CniIpam {
                    cni_type: "host-local".to_string(),
                    subnet: config.info.spec.subnet.clone(),
                    routes: config
                        .info
                        .spec
                        .routes
                        .iter()
                        .map(|route| CniIpamRoute {
                            destination: route.destination.clone(),
                        })
                        .collect(),
                },
            }],
        }
    }

    fn flannel_conf(config: &PlannedNetworkConfig) -> CniNetConfList {
        let flannel = config.info.spec.flannel.as_ref();
        let default_gateway = flannel
            .and_then(|settings| settings.default_gateway)
            .unwrap_or(config.info.spec.cluster_network.unwrap_or(true));

        let mut plugins = vec![CniConfContent::Flannel {
            subnet_file: flannel.and_then(|settings| option_if_not_empty(&settings.subnet_file)),
            data_dir: flannel.and_then(|settings| option_if_not_empty(&settings.data_dir)),
            delegate: Some(CniFlannelDelegate {
                bridge: Some(config.bridge.clone()),
                is_gateway: Some(default_gateway),
                is_default_gateway: Some(default_gateway),
                ip_masquerade: Some(config.info.spec.internet_access.unwrap_or(false)),
                hairpin_mode: Some(
                    flannel
                        .and_then(|settings| settings.hairpin_mode)
                        .unwrap_or(true),
                ),
            }),
        }];

        if flannel
            .and_then(|settings| settings.port_mappings)
            .unwrap_or(false)
        {
            plugins.push(CniConfContent::Portmap {
                capabilities: CniPortmapCapabilities {
                    port_mappings: true,
                },
            });
        }

        CniNetConfList {
            header: CniConfHeader {
                cni_version: CNI_VERSION.to_string(),
                name: config.info.name.clone(),
            },
            plugins,
        }
    }
}

fn option_if_not_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn network_class_identifier(info: &NetworkClassInfo) -> String {
    match &info.namespace {
        Some(namespace) => format!("{namespace}/{}", info.name),
        None => info.name.clone(),
    }
}

fn mac_address(namespace: &Option<String>, name: &str, ship_id: &str) -> String {
    let ident = match &namespace {
        Some(ns) => format!("NetworkClass/{ns}/{name}/{ship_id}"),
        None => format!("ClusterNetworkClass/{name}/{ship_id}"),
    };
    let digest = sha2::Sha256::digest(ident.as_bytes());
    format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        digest[0], digest[1], digest[2]
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
    use crate::cni::{
        CniWrapper, NetworkClassInfo, PlannedNetworkConfig, create_bridge_name, mac_address,
    };
    use tugboat_cni_operator::TugboatCniOperator;
    use tugboat_cni_operator::{CniConfContent, CniFlannelDelegate, CniIpamRoute};
    use tugboat_resources::manifests::core::v1::{
        FlannelNetworkClass, NetworkClassRoute, NetworkClassSpec,
    };
    use tugboat_vm_runtime_interface::run::VmNetworkConfig;

    fn planned_config(spec: NetworkClassSpec) -> PlannedNetworkConfig {
        PlannedNetworkConfig {
            bridge: "br-test".to_string(),
            info: NetworkClassInfo {
                name: "test-network".to_string(),
                namespace: Some("default".to_string()),
                spec,
            },
            vm: VmNetworkConfig {
                iface_name: "eth0".to_string(),
                mac_address: "52:54:00:aa:bb:cc".to_string(),
            },
        }
    }

    #[test]
    fn test_create_bridge_name() {
        let actual = create_bridge_name(&Some("ns".to_string()), "test");
        assert!(actual.len() < 15); // Linux's interface name length is 15 characters or fewer.
        assert_eq!(actual, "br-ns-8ead8051");

        let actual = create_bridge_name(&None, "test");
        assert!(actual.len() < 15); // Linux's interface name length is 15 characters or fewer.
        assert_eq!(actual, "br-cl-d5b95190");
    }

    #[test]
    fn default_plugin_uses_bridge_conflist() {
        let conf = CniWrapper::network_conf(&planned_config(NetworkClassSpec {
            subnet: "10.42.0.0/24".to_string(),
            routes: vec![NetworkClassRoute {
                destination: "0.0.0.0/0".to_string(),
            }],
            cluster_network: Some(true),
            ..Default::default()
        }))
        .unwrap();

        assert_eq!(conf.plugins.len(), 1);
        assert!(matches!(
            &conf.plugins[0],
            CniConfContent::Bridge {
                bridge,
                is_gateway,
                ip_masquerade,
                ipam,
            } if bridge == "br-test"
                && *is_gateway
                && !*ip_masquerade
                && ipam.subnet == "10.42.0.0/24"
                && ipam.routes == vec![CniIpamRoute {
                    destination: "0.0.0.0/0".to_string(),
                }]
        ));
    }

    #[test]
    fn flannel_plugin_generates_flannel_conflist() {
        let conf = CniWrapper::network_conf(&planned_config(NetworkClassSpec {
            cni_plugin: "flannel".to_string(),
            internet_access: Some(true),
            flannel: Some(FlannelNetworkClass {
                subnet_file: "/run/flannel/subnet.env".to_string(),
                data_dir: "/run/flannel".to_string(),
                hairpin_mode: Some(true),
                default_gateway: Some(false),
                port_mappings: Some(true),
            }),
            ..Default::default()
        }))
        .unwrap();

        assert_eq!(conf.plugins.len(), 2);
        assert!(matches!(
            &conf.plugins[0],
            CniConfContent::Flannel {
                subnet_file,
                data_dir,
                delegate: Some(CniFlannelDelegate {
                    bridge,
                    is_gateway,
                    is_default_gateway,
                    ip_masquerade,
                    hairpin_mode,
                }),
            } if subnet_file.as_deref() == Some("/run/flannel/subnet.env")
                && data_dir.as_deref() == Some("/run/flannel")
                && bridge.as_deref() == Some("br-test")
                && *is_gateway == Some(false)
                && *is_default_gateway == Some(false)
                && *ip_masquerade == Some(true)
                && *hairpin_mode == Some(true)
        ));
        assert!(matches!(
            &conf.plugins[1],
            CniConfContent::Portmap { capabilities } if capabilities.port_mappings
        ));
    }

    #[test]
    fn unsupported_plugin_is_rejected() {
        let err = CniWrapper::network_conf(&planned_config(NetworkClassSpec {
            cni_plugin: "bogus".to_string(),
            ..Default::default()
        }))
        .unwrap_err();

        assert!(matches!(
            err,
            tugboat_cni_operator::Error::InvalidConfiguration(_)
        ));
    }

    #[test]
    fn mac_address_is_stable_for_same_ship_and_network() {
        let namespace = Some("default".to_string());
        let first = mac_address(&namespace, "frontend", "ship-123");
        let second = mac_address(&namespace, "frontend", "ship-123");

        assert_eq!(first, second);
    }

    #[test]
    fn mac_address_uses_six_hex_octets() {
        let mac = mac_address(&Some("default".to_string()), "frontend", "ship-123");
        let parts = mac.split(':').collect::<Vec<_>>();

        assert_eq!(parts, ["52", "54", "00", parts[3], parts[4], parts[5]]);
        assert_eq!(parts.len(), 6);
        assert!(
            parts
                .iter()
                .all(|part| part.len() == 2 && part.chars().all(|ch| ch.is_ascii_hexdigit()))
        );
    }

    #[test]
    fn network_plan_preserves_guest_nic_identity_for_migration() {
        let wrapper = CniWrapper::new(TugboatCniOperator::new(
            toml::from_str(
                r#"
[location]
bin = "/tmp"
config = "/tmp"
netns = "/tmp"
"#,
            )
            .expect("test config should parse"),
        ));
        let network_classes = vec![
            NetworkClassInfo {
                name: "frontend".to_string(),
                namespace: Some("default".to_string()),
                spec: NetworkClassSpec {
                    subnet: "10.42.0.0/24".to_string(),
                    ..Default::default()
                },
            },
            NetworkClassInfo {
                name: "overlay".to_string(),
                namespace: None,
                spec: NetworkClassSpec {
                    cni_plugin: "flannel".to_string(),
                    flannel: Some(FlannelNetworkClass {
                        subnet_file: "/run/flannel/subnet.env".to_string(),
                        data_dir: "/run/flannel".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
        ];

        let first = wrapper.create_network_configs("ship-123", network_classes.clone());
        let second = wrapper.create_network_configs("ship-123", network_classes);

        let first_vm: Vec<_> = first.into_iter().map(|item| item.vm).collect();
        let second_vm: Vec<_> = second.into_iter().map(|item| item.vm).collect();
        assert_eq!(first_vm.len(), second_vm.len());
        for (first_vm, second_vm) in first_vm.iter().zip(second_vm.iter()) {
            assert_eq!(first_vm.iface_name, second_vm.iface_name);
            assert_eq!(first_vm.mac_address, second_vm.mac_address);
        }
        assert_eq!(first_vm[0].iface_name, "eth0");
        assert_eq!(first_vm[1].iface_name, "eth1");
    }
}
