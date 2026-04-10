use config::ControllerManagerConfig;
use deployment::DeploymentController;
use fleet::FleetController;
use manager::TugboatControllerManager;
use namespace_default_service_account::NamespaceDefaultServiceAccountController;
use network_class_status::NetworkClassStatusController;
use pv_cleanup::PersistentVolumeCleanupController;
use pvc_provisioner::PvcProvisionerController;
use replicaset::ReplicaSetController;
use service_account_token_controller::ServiceAccountTokenController;
use tugboat_client::TugboatClient;
use tugboat_csi_operator::TugboatCsiOperator;

mod base;
mod change_classifier;
mod config;
mod deployment;
mod error;
mod fleet;
mod manager;
mod namespace_default_service_account;
mod network_class_status;
mod provisioning;
mod pv_cleanup;
mod pvc_provisioner;
mod replicaset;
mod service_account_token_controller;

pub async fn run_with_config_file(path: impl AsRef<std::path::Path>) {
    let config = ControllerManagerConfig::load_or_panic(path);
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
    tcm.add_controller(FleetController::new(client.clone()));
    tcm.add_controller(DeploymentController::new(client.clone()));
    tcm.add_controller(ReplicaSetController::new(client.clone()));
    tcm.add_controller(NamespaceDefaultServiceAccountController::new(
        client.clone(),
    ));
    tcm.add_controller(ServiceAccountTokenController::new(client.clone()));
    tcm.add_controller(PersistentVolumeCleanupController::new(
        client,
        csi_operator,
        config,
    ));
    tcm.setup().await;
    tcm.run().await;
}
