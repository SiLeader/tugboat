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
use tugboat_client::TugboatClient;

mod cache;
mod config;
pub mod framework;
mod leader_election;
pub mod plugins;
mod scheduler;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-scheduler config file",
        default_value = "/etc/tugboat/scheduler/config.toml"
    )]
    config: String,
}

pub async fn run() {
    let args = Args::parse();
    let config = config::SchedulerConfig::load_or_panic(args.config);
    tokio::select! {
        _ = run_with_loaded_config(config) => {}
        _ = wait_for_shutdown_signal() => {
            tracing::info!("Received shutdown signal. Stopping scheduler.");
        }
    }
    tracing::info!("Scheduler stopped.");
}

pub async fn run_with_config_file(path: impl AsRef<std::path::Path>) {
    let config = config::SchedulerConfig::load_or_panic(path);
    run_with_loaded_config(config).await;
}

async fn run_with_loaded_config(config: config::SchedulerConfig) {
    let client = TugboatClient::try_new(
        &config.apiserver.url,
        config.apiserver.auth,
        config.apiserver.tls,
    )
    .unwrap_or_else(|e| panic!("Failed to configure tugboat client: {e}"));

    let mut fw = framework::Framework::new();
    for name in &config.scheduler.plugins.filter {
        if let Some(plugin) = plugins::create_filter_plugin(name) {
            fw.add_filter_plugin(plugin);
        } else {
            tracing::warn!("Unknown filter plugin: {name}");
        }
    }
    for name in &config.scheduler.plugins.score {
        if let Some(plugin) = plugins::create_score_plugin(name) {
            fw.add_score_plugin(plugin);
        } else {
            tracing::warn!("Unknown score plugin: {name}");
        }
    }

    let leader_elector = leader_election::LeaderElector::new(
        client.clone(),
        config.scheduler.name.clone(),
        config.scheduler.lease_duration_seconds,
        config.scheduler.renew_interval_seconds,
    );

    let scheduler = scheduler::Scheduler::new(client, fw, leader_elector, config.scheduler);
    scheduler.run().await;
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::SignalKind;

        let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
            .expect("Failed to listen terminate signal");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen ctrl-c signal");
    }
}
