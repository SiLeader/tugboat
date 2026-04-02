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

mod error;
mod materialized_volume;
mod network;
pub(crate) mod ops;
mod reconcile;
mod volume;

pub(crate) use ops::ShipFingerprints;

use crate::cni::CniWrapper;
use crate::csi::{CsiDrivers, CsiWrapper};
use crate::reconciler::reconcile::AppendStatus;
use crate::reconciler::volume::{
    MaterializedVolumeSourceKind, ship_references_materialized_resource,
};
use crate::runtime::RuntimeOperator;
use std::path::PathBuf;
use std::time::Duration;
use tokio::select;
use tokio::signal::unix::SignalKind;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tugboat_client::runtime::Controller;
use tugboat_client::{Api, TugboatClient, WatchParams};
use tugboat_cni_operator::TugboatCniOperator;
use tugboat_csi_operator::TugboatCsiOperator;
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{ConfigMap, Secret, Ship, ShipClass};

#[derive(Clone)]
pub(crate) struct ShipReconciler {
    node_name: String,
    client: TugboatClient,
    ship_all_api: Api<Ship>,
    ship_class_api: Api<ShipClass>,
    runtime_operator: RuntimeOperator,
    cni: CniWrapper,
    csi: CsiWrapper,
    volume_data_dir: PathBuf,
    cancellation_token: CancellationToken,
}

impl ShipReconciler {
    pub(crate) fn new(
        node_name: String,
        client: TugboatClient,
        runtime_operator: RuntimeOperator,
        cni: TugboatCniOperator,
        csi: TugboatCsiOperator,
        csi_drivers: CsiDrivers,
        csi_publish_dir: String,
    ) -> Self {
        Self {
            node_name,
            ship_all_api: Api::all(client.clone()),
            ship_class_api: Api::all(client.clone()),
            client,
            runtime_operator,
            cni: CniWrapper::new(cni),
            csi: CsiWrapper::new(csi, csi_drivers, csi_publish_dir.clone()),
            volume_data_dir: PathBuf::from(csi_publish_dir),
            cancellation_token: CancellationToken::new(),
        }
    }

    pub(crate) async fn run(self) {
        info!(
            "Starting ship reconciliation loop on node '{}'",
            self.node_name
        );
        self.spawn_watch_shutdown_signal();
        self.start_status_collector();

        let ship_controller = Controller::new(self.ship_all_api.clone())
            .with_watch_params(
                WatchParams::default().fields(format!("spec.nodeName={}", self.node_name)),
            )
            .with_cancellation_token(self.cancellation_token.clone());
        let ship_target_controller = Controller::new(self.ship_all_api.clone())
            .with_watch_params(
                WatchParams::default().fields(format!("spec.targetNodeName={}", self.node_name)),
            )
            .with_cancellation_token(self.cancellation_token.clone());
        let config_map_controller = Controller::new(Api::<ConfigMap>::all(self.client.clone()))
            .with_cancellation_token(self.cancellation_token.clone());
        let secret_controller = Controller::new(Api::<Secret>::all(self.client.clone()))
            .with_cancellation_token(self.cancellation_token.clone());

        let ship_reconciler = {
            let this = self.clone();
            move |event| {
                let this = this.clone();
                async move { this.reconcile(event).await }
            }
        };
        let ship_target_reconciler = {
            let this = self.clone();
            move |event| {
                let this = this.clone();
                async move { this.reconcile(event).await }
            }
        };
        let config_map_reconciler = {
            let this = self.clone();
            move |event| {
                let this = this.clone();
                async move { this.reconcile_config_map(event).await }
            }
        };
        let secret_reconciler = {
            let this = self.clone();
            move |event| {
                let this = this.clone();
                async move { this.reconcile_secret(event).await }
            }
        };

        tokio::join!(
            ship_controller.run(ship_reconciler),
            ship_target_controller.run(ship_target_reconciler),
            config_map_controller.run(config_map_reconciler),
            secret_controller.run(secret_reconciler)
        );
        info!("Ship reconciliation loop stopped.");
    }

    async fn reconcile_config_map(
        &self,
        event: tugboat_client::runtime::ReconcileEvent<ConfigMap>,
    ) -> Result<tugboat_client::runtime::Action, crate::reconciler::error::ReconcileError> {
        // Skip refresh when the ConfigMap is deleted — the volume files are
        // already absent and attempting to re-materialize a missing resource
        // would only produce errors.
        let config_map = match event {
            tugboat_client::runtime::ReconcileEvent::Applied(config_map) => config_map,
            tugboat_client::runtime::ReconcileEvent::Deleted(_) => {
                return Ok(tugboat_client::runtime::Action::await_change());
            }
        };
        self.reconcile_materialized_resource_change(
            &config_map,
            MaterializedVolumeSourceKind::ConfigMap,
        )
        .await?;
        Ok(tugboat_client::runtime::Action::await_change())
    }

    async fn reconcile_secret(
        &self,
        event: tugboat_client::runtime::ReconcileEvent<Secret>,
    ) -> Result<tugboat_client::runtime::Action, crate::reconciler::error::ReconcileError> {
        // Skip refresh when the Secret is deleted — same reasoning as ConfigMap.
        let secret = match event {
            tugboat_client::runtime::ReconcileEvent::Applied(secret) => secret,
            tugboat_client::runtime::ReconcileEvent::Deleted(_) => {
                return Ok(tugboat_client::runtime::Action::await_change());
            }
        };
        self.reconcile_materialized_resource_change(&secret, MaterializedVolumeSourceKind::Secret)
            .await?;
        Ok(tugboat_client::runtime::Action::await_change())
    }

    async fn reconcile_materialized_resource_change<T>(
        &self,
        resource: &T,
        kind: MaterializedVolumeSourceKind,
    ) -> Result<(), crate::reconciler::error::ReconcileError>
    where
        T: ObjectMetaResource,
    {
        let Some(name) = resource.name() else {
            return Err(crate::reconciler::error::ReconcileError::FieldMissing(
                kind.as_str().to_string(),
                "metadata.name".to_string(),
            ));
        };
        let namespace = resource.namespace().unwrap_or("default");
        let ships = self
            .ships_referencing_materialized_resource(namespace, kind, name)
            .await?;
        if ships.is_empty() {
            return Ok(());
        }

        info!(
            "{} '{}/{}' changed; refreshing materialized volumes for {} ship(s)",
            kind.as_str(),
            namespace,
            name,
            ships.len()
        );

        let mut first_error = None;
        for ship in ships {
            let ship_name = ship.name().unwrap_or("<unknown>").to_string();
            if let Err(err) = self.refresh_materialized_volumes_for_ship(ship).await {
                warn!(
                    "Failed to refresh materialized volumes for Ship '{}/{}' after {} '{}/{}' changed: {}",
                    namespace,
                    ship_name,
                    kind.as_str(),
                    namespace,
                    name,
                    err
                );
                if first_error.is_none() {
                    first_error = Some(err);
                }
            } else {
                debug!(
                    "Refreshed materialized volumes for Ship '{}/{}' after {} '{}/{}' changed",
                    namespace,
                    ship_name,
                    kind.as_str(),
                    namespace,
                    name
                );
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        Ok(())
    }

    async fn ships_referencing_materialized_resource(
        &self,
        namespace: &str,
        kind: MaterializedVolumeSourceKind,
        resource_name: &str,
    ) -> Result<Vec<Ship>, crate::reconciler::error::ReconcileError> {
        // Query ships owned by this node
        let mut ships = self
            .ship_all_api
            .list_with_params(
                &WatchParams::default().fields(format!("spec.nodeName={}", self.node_name)),
            )
            .await?;
            
        // Also query ships migrating to this node
        if let Ok(mut target_ships) = self
            .ship_all_api
            .list_with_params(
                &WatchParams::default().fields(format!("spec.targetNodeName={}", self.node_name)),
            )
            .await
        {
            ships.append(&mut target_ships);
        }

        let mut matched = Vec::new();

        for ship in ships {
            if ship.namespace().unwrap_or("default") != namespace {
                continue;
            }

            let ship_name = ship.name().unwrap_or("<unknown>").to_string();
            let Some(spec) = ship.spec.as_ref() else {
                warn!(
                    "Skipping Ship '{}/{}' while reconciling {} '{}/{}' because spec is missing",
                    namespace,
                    ship_name,
                    kind.as_str(),
                    namespace,
                    resource_name
                );
                continue;
            };

            match ship_references_materialized_resource(spec, kind, resource_name) {
                Ok(true) => matched.push(ship),
                Ok(false) => {}
                Err(err) => {
                    warn!(
                        "Failed to inspect Ship '{}/{}' while reconciling {} '{}/{}': {}",
                        namespace,
                        ship_name,
                        kind.as_str(),
                        namespace,
                        resource_name,
                        err
                    );
                }
            }
        }

        Ok(matched)
    }

    fn start_status_collector(&self) -> JoinHandle<()> {
        let client = self.client.clone();
        let operator = self.runtime_operator.clone();
        let token = self.cancellation_token.clone();
        tokio::spawn(async move {
            loop {
                for status in operator.collect_status().await {
                    match status {
                        Ok(status) => {
                            let api: Api<Ship> = Api::namespaced(client.clone(), &status.namespace);
                            let mut ship = match api.get(&status.ship_name).await {
                                Ok(Some(s)) => s,
                                Ok(None) => continue,
                                Err(err) => {
                                    error!("Failed to get ship: {err}");
                                    continue;
                                }
                            };

                            ship.append_status(status.condition);
                            if let Err(e) = api.replace_status(&status.ship_name, ship).await {
                                error!("Failed to update ship status: {e}");
                            }
                        }
                        Err(err) => {
                            error!("Collecting ship status failed: {err}");
                        }
                    }
                }
                select! {
                    _ = sleep(Duration::from_secs(5)) => {}
                    _ = token.cancelled() => {
                        break;
                    }
                }
            }
        })
    }

    fn spawn_watch_shutdown_signal(&self) {
        let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
            .expect("Failed to listen terminate signal");
        let token = self.cancellation_token.clone();
        tokio::spawn(async move {
            select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            info!("Received terminate signal. Shutting down ship reconciler.");
            token.cancel();
        });
    }
}
