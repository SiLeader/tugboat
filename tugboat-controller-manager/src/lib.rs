use aggregated_clusterrole::AggregatedClusterRoleController;
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
use ship_snapshot_volumes::ShipSnapshotVolumesController;
use tugboat_client::TugboatClient;
use tugboat_csi_operator::TugboatCsiOperator;
use volume_snapshot::VolumeSnapshotController;

mod aggregated_clusterrole;
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
mod ship_snapshot_volumes;
mod volume_snapshot;
mod workload;

pub async fn run_with_config_file(path: impl AsRef<std::path::Path>) {
    let config = ControllerManagerConfig::load(path).unwrap_or_else(|e| panic!("{e}"));
    let client = TugboatClient::try_new(
        config.apiserver.url.clone(),
        config.apiserver.auth.clone(),
        config.apiserver.tls.clone(),
    )
    .unwrap_or_else(|e| {
        panic!("tugboat-controller-manager failed to configure tugboat client: {e}")
    });
    let csi_operator = TugboatCsiOperator::with_timeouts(config.csi.timeouts());

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
    tcm.add_controller(VolumeSnapshotController::new(
        client.clone(),
        csi_operator.clone(),
        config.clone(),
    ));
    tcm.add_controller(ShipSnapshotVolumesController::new(client.clone()));
    tcm.add_controller(FleetController::new(client.clone()));
    tcm.add_controller(DeploymentController::new(client.clone()));
    tcm.add_controller(ReplicaSetController::new(client.clone()));
    tcm.add_controller(NamespaceDefaultServiceAccountController::new(
        client.clone(),
    ));
    tcm.add_controller(ServiceAccountTokenController::new(client.clone()));
    tcm.add_controller(AggregatedClusterRoleController::new(client.clone()));
    tcm.add_controller(PersistentVolumeCleanupController::new(
        client,
        csi_operator,
        config,
    ));
    tcm.setup().await;
    tcm.run().await;
}
