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
mod network;
mod ops;
mod reconcile;

use crate::cni::CniWrapper;
use crate::reconciler::reconcile::AppendStatus;
use crate::runtime::RuntimeOperator;
use futures::{Stream, StreamExt};
use std::cmp::min;
use std::collections::HashSet;
use std::pin::Pin;
use std::time::Duration;
use tokio::select;
use tokio::signal::unix::SignalKind;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tugboat_client::{Api, TugboatClient, WatchEvent, WatchParams};
use tugboat_cni_operator::CniOperator;
use tugboat_resources::manifests::core::v1::{Ship, ShipClass};

#[derive(Clone)]
pub(crate) struct ShipReconciler {
    node_name: String,
    client: TugboatClient,
    ship_all_api: Api<Ship>,
    ship_class_api: Api<ShipClass>,
    runtime_operator: RuntimeOperator,
    cni: CniWrapper,
    cancellation_token: CancellationToken,
}

impl ShipReconciler {
    pub(crate) fn new(
        node_name: String,
        client: TugboatClient,
        runtime_operator: RuntimeOperator,
        cni: CniOperator,
    ) -> Self {
        Self {
            node_name,
            ship_all_api: Api::all(client.clone()),
            ship_class_api: Api::all(client.clone()),
            client,
            runtime_operator,
            cni: CniWrapper::new(cni),
            cancellation_token: CancellationToken::new(),
        }
    }

    pub(crate) async fn run(self) {
        info!(
            "Starting ship reconciliation loop on node '{}'",
            self.node_name
        );
        self.spawn_watch_shutdown_signal();

        let watch_params =
            WatchParams::default().fields(format!("spec.nodeName={}", self.node_name));
        self.start_status_collector();

        let Some(mut stream) = self.get_watch_stream(&watch_params).await else {
            info!("Get watch stream stopped. Shutting down ship reconciler.");
            return;
        };
        loop {
            let event = select! {
                event = stream.next() => event,
                _ = self.cancellation_token.cancelled() => {
                    info!("Ship reconciliation loop stopped.");
                    break;
                }
            };
            let Some(event) = event else {
                continue;
            };
            match event {
                Ok(event) => {
                    if let Err(e) = self.reconcile(event).await {
                        error!("Failed to reconcile ship: {e}");
                    }
                }
                Err(err) => {
                    error!("Failed to watch ship: {err}");
                }
            }
        }
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
                            let mut ship = match api.get(&status.id).await {
                                Ok(Some(s)) => s,
                                Ok(None) => continue,
                                Err(err) => {
                                    error!("Failed to get ship: {err}");
                                    continue;
                                }
                            };

                            ship.append_status(status.condition);
                            if let Err(e) = api.replace_status(&status.id, ship).await {
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

    async fn get_watch_stream(
        &self,
        params: &WatchParams,
    ) -> Option<Pin<Box<impl Stream<Item = Result<WatchEvent<Ship>, tugboat_client::Error>>>>> {
        let mut count = 0;
        loop {
            match self.ship_all_api.watch(params).await {
                Ok(stream) => return Some(Box::pin(stream)),
                Err(e) => {
                    error!("Failed to create watch stream: {e}");
                    select! {
                        _ = self.cancellation_token.cancelled() => {
                            return None;
                        }
                        _ = sleep(Duration::from_secs(min(128, 2u64.pow(count)))) => {}
                    }
                    count += 1;
                }
            }
        }
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
