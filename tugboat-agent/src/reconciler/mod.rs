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

mod dependency;
mod error;
mod materialized_volume;
mod network;
pub(crate) mod ops;
mod reconcile;
mod runner;
mod volume;

pub(crate) use ops::ShipFingerprints;

use crate::cni::CniWrapper;
use crate::csi::{CsiDrivers, CsiWrapper};
use crate::reconciler::reconcile::AppendStatus;
use crate::runtime::RuntimeOperator;
use runner::ReconcilerRunner;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::select;
use tokio::signal::unix::SignalKind;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tugboat_client::{Api, TugboatClient};
use tugboat_cni_operator::TugboatCniOperator;
use tugboat_csi_operator::TugboatCsiOperator;

use tugboat_resources::manifests::core::v1::{Ship, ShipClass};

/// Key identifying a single projected ServiceAccount token refresh task. We
/// dedupe on (ship_id, volume_name, path) because a Ship can be re-reconciled
/// (add → restart → add) multiple times for the same projected file, and each
/// call to `start_service_account_token_refresh` would otherwise spawn a fresh
/// task that races the existing one for atomic-rename writes.
pub(crate) type TokenRefreshKey = (String, String, PathBuf);

#[derive(Clone, Default)]
pub(crate) struct TokenRefreshRegistry {
    tasks: Arc<Mutex<HashMap<TokenRefreshKey, CancellationToken>>>,
}

impl TokenRefreshRegistry {
    /// Inserts a fresh `CancellationToken` for `key`, cancelling any previous
    /// task registered under the same key. Returns the new token the caller
    /// should hand to the spawned task.
    pub(crate) fn replace(&self, key: TokenRefreshKey) -> CancellationToken {
        let new_token = CancellationToken::new();
        let mut tasks = self.tasks.lock().expect("token refresh registry poisoned");
        if let Some(previous) = tasks.insert(key, new_token.clone()) {
            previous.cancel();
        }
        new_token
    }

    pub(crate) fn forget(&self, key: &TokenRefreshKey) {
        let mut tasks = self.tasks.lock().expect("token refresh registry poisoned");
        tasks.remove(key);
    }
}

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
    apiserver_ca_cert_path: Option<PathBuf>,
    cancellation_token: CancellationToken,
    pub(crate) token_refreshes: TokenRefreshRegistry,
}

impl ShipReconciler {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        node_name: String,
        client: TugboatClient,
        runtime_operator: RuntimeOperator,
        cni: TugboatCniOperator,
        csi: TugboatCsiOperator,
        csi_drivers: CsiDrivers,
        csi_publish_dir: String,
        apiserver_ca_cert_path: Option<String>,
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
            apiserver_ca_cert_path: apiserver_ca_cert_path.map(PathBuf::from),
            cancellation_token: CancellationToken::new(),
            token_refreshes: TokenRefreshRegistry::default(),
        }
    }

    pub(crate) async fn run(self) {
        let runner = ReconcilerRunner::new(self);
        runner.run().await;
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
                            let ship = match api.get(&status.ship_name).await {
                                Ok(Some(s)) => s,
                                Ok(None) => continue,
                                Err(err) => {
                                    error!("Failed to get ship: {err}");
                                    continue;
                                }
                            };

                            let mut conditions =
                                ship.status.map(|s| s.conditions).unwrap_or_default();
                            conditions.append_status(status.condition);
                            let patch = serde_json::json!({
                                "status": {
                                    "conditions": conditions
                                }
                            });
                            if let Err(e) = api.patch_status(&status.ship_name, patch).await {
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
