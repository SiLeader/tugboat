use crate::base::TugboatController;
use crate::config::ControllerManagerConfig;
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, NetworkClass, NetworkClassCondition, NetworkClassSpec, NetworkClassStatus,
    Node, NodeCniPluginStatus,
};
use tugboat_resources::manifests::meta::v1::Time;

pub(crate) struct NetworkClassStatusController {
    client: TugboatClient,
    config: ControllerManagerConfig,
}

impl NetworkClassStatusController {
    pub(crate) fn new(client: TugboatClient, config: ControllerManagerConfig) -> Self {
        Self { client, config }
    }

    async fn reconcile_all(&self) -> Result<(), tugboat_client::Error> {
        let node_api: Api<Node> = Api::all(self.client.clone());
        let cluster_network_class_api: Api<ClusterNetworkClass> = Api::all(self.client.clone());
        let network_class_api: Api<NetworkClass> = Api::all(self.client.clone());

        let nodes = node_api.list().await?;

        for mut network_class in cluster_network_class_api.list().await? {
            let Some(name) = network_class
                .object_meta
                .as_ref()
                .and_then(|meta| meta.name.clone())
            else {
                tracing::warn!("Skipping ClusterNetworkClass without metadata.name");
                continue;
            };
            network_class.status = Some(build_network_class_status(
                network_class.spec.as_ref(),
                &nodes,
            ));
            cluster_network_class_api
                .replace_status(&name, network_class)
                .await?;
        }

        for mut network_class in network_class_api.list().await? {
            let Some(meta) = network_class.object_meta.as_ref() else {
                tracing::warn!("Skipping NetworkClass without metadata");
                continue;
            };
            let Some(name) = meta.name.clone() else {
                tracing::warn!("Skipping NetworkClass without metadata.name");
                continue;
            };
            let Some(namespace) = meta.namespace.clone() else {
                tracing::warn!(
                    "Skipping NetworkClass '{}' without metadata.namespace",
                    name
                );
                continue;
            };

            network_class.status = Some(build_network_class_status(
                network_class.spec.as_ref(),
                &nodes,
            ));
            let network_class_api: Api<NetworkClass> =
                Api::namespaced(self.client.clone(), &namespace);
            network_class_api
                .replace_status(&name, network_class)
                .await?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TugboatController for NetworkClassStatusController {
    fn name(&self) -> &str {
        "networkclass-status"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        let interval = self.config.network.requeue_interval();
        loop {
            if let Err(err) = self.reconcile_all().await {
                tracing::warn!("Failed to reconcile network class status: {}", err);
            }
            tokio::time::sleep(interval).await;
        }
    }
}

fn build_network_class_status(
    spec: Option<&NetworkClassSpec>,
    nodes: &[Node],
) -> NetworkClassStatus {
    let timestamp = Some(Time::now());
    let Ok(required_plugins) = required_plugins(spec) else {
        let reason = required_plugins(spec).unwrap_err();
        return NetworkClassStatus {
            conditions: vec![
                NetworkClassCondition {
                    r#type: "Accepted".to_string(),
                    status: "False".to_string(),
                    message: reason.clone(),
                    timestamp,
                },
                NetworkClassCondition {
                    r#type: "Ready".to_string(),
                    status: "False".to_string(),
                    message: reason,
                    timestamp,
                },
            ],
            ready_nodes: Vec::new(),
        };
    };

    let ready_nodes = nodes
        .iter()
        .filter(|node| node_supports_plugins(node, &required_plugins))
        .filter_map(|node| node.object_meta.as_ref().and_then(|meta| meta.name.clone()))
        .collect::<Vec<_>>();

    let ready_message = if ready_nodes.is_empty() {
        format!(
            "No node currently advertises the required CNI plugins: {}.",
            required_plugins.join(", ")
        )
    } else {
        format!(
            "{} node(s) advertise the required CNI plugins: {}.",
            ready_nodes.len(),
            ready_nodes.join(", ")
        )
    };

    NetworkClassStatus {
        conditions: vec![
            NetworkClassCondition {
                r#type: "Accepted".to_string(),
                status: "True".to_string(),
                message: format!(
                    "NetworkClass uses supported cniPlugin '{}'.",
                    normalized_plugin(spec.expect("spec checked above"))
                ),
                timestamp,
            },
            NetworkClassCondition {
                r#type: "Ready".to_string(),
                status: if ready_nodes.is_empty() {
                    "False".to_string()
                } else {
                    "True".to_string()
                },
                message: ready_message,
                timestamp,
            },
        ],
        ready_nodes,
    }
}

fn required_plugins(spec: Option<&NetworkClassSpec>) -> Result<Vec<&'static str>, String> {
    let Some(spec) = spec else {
        return Err("NetworkClass spec is missing.".to_string());
    };

    match normalized_plugin(spec) {
        "bridge" => Ok(vec!["bridge", "loopback"]),
        "flannel" => {
            let mut plugins = vec!["bridge", "flannel", "loopback"];
            if spec
                .flannel
                .as_ref()
                .and_then(|flannel| flannel.port_mappings)
                .unwrap_or(false)
            {
                plugins.push("portmap");
            }
            Ok(plugins)
        }
        other => Err(format!(
            "Unsupported cniPlugin '{}'. Supported values are bridge and flannel.",
            other
        )),
    }
}

fn normalized_plugin(spec: &NetworkClassSpec) -> &str {
    let plugin = spec.cni_plugin.trim();
    if plugin.is_empty() { "bridge" } else { plugin }
}

fn node_supports_plugins(node: &Node, required_plugins: &[&str]) -> bool {
    let Some(status) = node.status.as_ref() else {
        return false;
    };

    required_plugins
        .iter()
        .all(|plugin| plugin_ready(&status.cni_plugins, plugin))
}

fn plugin_ready(statuses: &[NodeCniPluginStatus], plugin: &str) -> bool {
    statuses
        .iter()
        .find(|status| status.name == plugin)
        .and_then(|status| status.ready)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{FlannelNetworkClass, NodeStatus};
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn default_network_class_is_ready_when_node_has_bridge_and_loopback() {
        let status = build_network_class_status(
            Some(&NetworkClassSpec::default()),
            &[node_with_plugins(&[("bridge", true), ("loopback", true)])],
        );

        assert_eq!(status.conditions[0].status, "True");
        assert_eq!(status.conditions[1].status, "True");
        assert_eq!(status.ready_nodes, vec!["node-a".to_string()]);
    }

    #[test]
    fn flannel_network_class_requires_portmap_when_requested() {
        let status = build_network_class_status(
            Some(&NetworkClassSpec {
                cni_plugin: "flannel".to_string(),
                flannel: Some(FlannelNetworkClass {
                    port_mappings: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            &[node_with_plugins(&[
                ("bridge", true),
                ("loopback", true),
                ("flannel", true),
            ])],
        );

        assert_eq!(status.conditions[1].status, "False");
        assert!(status.conditions[1].message.contains("portmap"));
    }

    #[test]
    fn unsupported_plugin_is_rejected() {
        let status = build_network_class_status(
            Some(&NetworkClassSpec {
                cni_plugin: "cilium".to_string(),
                ..Default::default()
            }),
            &[],
        );

        assert_eq!(status.conditions[0].status, "False");
        assert!(status.conditions[0].message.contains("Unsupported"));
    }

    fn node_with_plugins(plugins: &[(&str, bool)]) -> Node {
        Node {
            object_meta: Some(ObjectMeta {
                name: Some("node-a".to_string()),
                ..Default::default()
            }),
            status: Some(NodeStatus {
                cni_plugins: plugins
                    .iter()
                    .map(|(name, ready)| NodeCniPluginStatus {
                        name: (*name).to_string(),
                        ready: Some(*ready),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
