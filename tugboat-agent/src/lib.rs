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
    let config = config::AgentConfig::load_or_panic(args.config);

    let client = TugboatClient::new(config.apiserver.url);
    node_registration::ensure_node_exists(client.clone(), config.node.name.clone())
        .await
        .unwrap_or_else(|e| panic!("Failed to ensure node resource exists: {e}"));
    let runtime_operator = RuntimeOperator::new(
        config.runtime,
        config.image.cache_dir,
        config.image.http_hosts,
    );
    let csi_operator = tugboat_csi_operator::TugboatCsiOperator::default();
    let cni_operator = tugboat_cni_operator::TugboatCniOperator::new(config.cni);

    cni_operator
        .initialize()
        .await
        .unwrap_or_else(|e| panic!("Failed to initialize CNI operator: {e}"));

    let reconciler = ShipReconciler::new(
        config.node.name,
        client,
        runtime_operator,
        cni_operator,
        csi_operator,
        CsiDrivers::default(),
    );

    reconciler.run().await;
}
