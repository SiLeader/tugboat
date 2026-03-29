use crate::config::ControllerManagerConfig;
use crate::manager::TugboatControllerManager;
use crate::network_class_status::NetworkClassStatusController;
use crate::pv_cleanup::PersistentVolumeCleanupController;
use crate::pvc_provisioner::PvcProvisionerController;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use tugboat_client::TugboatClient;
use tugboat_csi_operator::TugboatCsiOperator;

mod base;
mod config;
mod error;
mod manager;
mod network_class_status;
mod provisioning;
mod pv_cleanup;
mod pvc_provisioner;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-controller-manager config file",
        default_value = "/etc/tugboat/controller-manager/config.toml"
    )]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let config = ControllerManagerConfig::load_or_panic(args.config);
    let client = TugboatClient::new(config.apiserver.url.clone());
    let csi_operator = TugboatCsiOperator::default();

    let mut tcm = TugboatControllerManager::new();
    tcm.add_controller(NetworkClassStatusController::new(
        client.clone(),
        config.clone(),
    ));
    tcm.add_controller(PvcProvisionerController::new(
        client.clone(),
        csi_operator.clone(),
        config.clone(),
    ));
    tcm.add_controller(PersistentVolumeCleanupController::new(
        client,
        csi_operator,
        config,
    ));
    tcm.setup().await;
    tcm.run().await;
}
