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
mod ops;
mod reconcile;
mod volume;

use crate::cni::CniWrapper;
use crate::csi::{CsiDrivers, CsiWrapper};
use crate::reconciler::reconcile::AppendStatus;
use crate::runtime::RuntimeOperator;
use std::path::PathBuf;
use std::time::Duration;
use tokio::select;
use tokio::signal::unix::SignalKind;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tugboat_client::runtime::Controller;
use tugboat_client::{Api, TugboatClient, WatchParams};
use tugboat_cni_operator::TugboatCniOperator;
use tugboat_csi_operator::TugboatCsiOperator;
use tugboat_resources::manifests::core::v1::{Ship, ShipClass};

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

        let controller = Controller::new(self.ship_all_api.clone())
            .with_watch_params(
                WatchParams::default().fields(format!("spec.nodeName={}", self.node_name)),
            )
            .with_cancellation_token(self.cancellation_token.clone());
        let reconciler = {
            let this = self.clone();
            move |event| {
                let this = this.clone();
                async move { this.reconcile(event).await }
            }
        };

        controller.run(reconciler).await;
        info!("Ship reconciliation loop stopped.");
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
