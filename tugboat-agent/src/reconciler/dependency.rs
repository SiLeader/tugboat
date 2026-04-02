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

use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::{MaterializedVolumeSourceKind, ship_references_materialized_resource};
use tokio::sync::mpsc;
use tracing::warn;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent};
use tugboat_client::{Api, TugboatClient, WatchParams};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{ConfigMap, Secret, Ship};

#[derive(Debug)]
pub(crate) enum DependencyEvent {
    ResourceChanged {
        kind: MaterializedVolumeSourceKind,
        namespace: String,
        name: String,
    },
}

#[derive(Clone)]
pub(crate) struct DependencyTracker {
    node_name: String,
    client: TugboatClient,
    ship_all_api: Api<Ship>,
    event_tx: mpsc::Sender<DependencyEvent>,
}

impl DependencyTracker {
    pub(crate) fn new(
        node_name: String,
        client: TugboatClient,
        event_tx: mpsc::Sender<DependencyEvent>,
    ) -> Self {
        Self {
            node_name,
            ship_all_api: Api::all(client.clone()),
            client,
            event_tx,
        }
    }

    pub(crate) fn build_config_map_controller(&self) -> Controller<ConfigMap> {
        Controller::new(Api::all(self.client.clone()))
    }

    pub(crate) fn build_secret_controller(&self) -> Controller<Secret> {
        Controller::new(Api::all(self.client.clone()))
    }

    pub(crate) async fn handle_config_map_event(
        &self,
        event: ReconcileEvent<ConfigMap>,
    ) -> Result<Action, ReconcileError> {
        let config_map = match event {
            ReconcileEvent::Applied(cm) => cm,
            ReconcileEvent::Deleted(_) => return Ok(Action::await_change()),
        };

        let name = config_map.name().ok_or_else(|| {
            ReconcileError::FieldMissing("ConfigMap".to_string(), "metadata.name".to_string())
        })?;
        let namespace = config_map.namespace().unwrap_or("default").to_string();

        let _ = self.event_tx.send(DependencyEvent::ResourceChanged {
            kind: MaterializedVolumeSourceKind::ConfigMap,
            namespace,
            name: name.to_string(),
        }).await;

        Ok(Action::await_change())
    }

    pub(crate) async fn handle_secret_event(
        &self,
        event: ReconcileEvent<Secret>,
    ) -> Result<Action, ReconcileError> {
        let secret = match event {
            ReconcileEvent::Applied(s) => s,
            ReconcileEvent::Deleted(_) => return Ok(Action::await_change()),
        };

        let name = secret.name().ok_or_else(|| {
            ReconcileError::FieldMissing("Secret".to_string(), "metadata.name".to_string())
        })?;
        let namespace = secret.namespace().unwrap_or("default").to_string();

        let _ = self.event_tx.send(DependencyEvent::ResourceChanged {
            kind: MaterializedVolumeSourceKind::Secret,
            namespace,
            name: name.to_string(),
        }).await;

        Ok(Action::await_change())
    }

    pub(crate) async fn ships_referencing_resource(
        &self,
        namespace: &str,
        kind: MaterializedVolumeSourceKind,
        resource_name: &str,
    ) -> Result<Vec<Ship>, ReconcileError> {
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
}
