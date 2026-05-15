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

use crate::csi::CsiDrivers;
use crate::reconciler::ShipReconciler;
use crate::runtime::RuntimeOperator;
use clap::Parser;
use tugboat_client::TugboatClient;

mod cni;
mod config;
mod csi;
mod mountns;
mod node_registration;
mod reconciler;
mod runtime;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-agent config file",
        default_value = "/etc/tugboat/agent/config.toml"
    )]
    config: String,
}

pub async fn run() {
    let args = Args::parse();
    let config = match config::AgentConfig::load(args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tugboat-agent failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    let node_name = config.node.name.clone();
    let runtime_class = config.node.runtime_class.clone();
    let topology = config.topology.clone();
    let image_cache_dir = config.image.cache_dir.clone();
    let network_probe_interval = config.node.network_probe_interval();
    let cni_config = config.cni.clone();
    let apiserver_ca_cert_path = config.apiserver.tls.ca_cert_path.clone();
    let client = TugboatClient::try_new(
        config.apiserver.url,
        config.apiserver.auth,
        config.apiserver.tls,
    )
    .unwrap_or_else(|e| {
        eprintln!("tugboat-agent failed to configure tugboat client: {e}");
        std::process::exit(1);
    });
    node_registration::ensure_node_exists(
        client.clone(),
        node_name.clone(),
        runtime_class.clone(),
        &topology,
    )
    .await
    .unwrap_or_else(|e| {
        eprintln!("tugboat-agent failed to ensure node resource exists: {e}");
        std::process::exit(1);
    });
    let runtime_operator = RuntimeOperator::new(
        config.runtime,
        image_cache_dir.clone(),
        config.image.http_hosts,
    );
    let csi_operator =
        tugboat_csi_operator::TugboatCsiOperator::with_timeouts(config.csi.timeouts());
    let cni_operator = tugboat_cni_operator::TugboatCniOperator::new(config.cni);

    cni_operator.initialize().await.unwrap_or_else(|e| {
        eprintln!("tugboat-agent failed to initialize CNI operator: {e}");
        std::process::exit(1);
    });

    node_registration::publish_node_status(
        client.clone(),
        &node_name,
        runtime_class.as_deref(),
        &topology,
        &cni_config,
        &image_cache_dir,
    )
    .await
    .unwrap_or_else(|e| {
        eprintln!("tugboat-agent failed to publish node CNI status: {e}");
        std::process::exit(1);
    });
    tokio::spawn(node_registration::refresh_node_status_loop(
        client.clone(),
        node_name.clone(),
        runtime_class,
        topology,
        cni_config,
        image_cache_dir,
        network_probe_interval,
    ));

    let reconciler = ShipReconciler::new(
        node_name,
        client,
        runtime_operator,
        cni_operator,
        csi_operator,
        CsiDrivers::from(config.csi.drivers),
        config.csi.publish_dir,
        apiserver_ca_cert_path,
    );

    reconciler.run().await;
}
