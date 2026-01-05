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

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tugboat_apiserver::ApiServer;
use tugboat_apiserver::config::ApiServerConfig;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "/etc/tugboat/apiserver/config.toml")]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    info!("Starting Tugboat API server");

    let config = ApiServerConfig::load_from_file_or_panic(args.config);
    ApiServer::from_config(config).await.run().await;
}
