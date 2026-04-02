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

use crate::reconciler::ShipReconciler;
use crate::reconciler::dependency::{DependencyEvent, DependencyTracker};
use tokio::select;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use tugboat_client::WatchParams;
use tugboat_client::runtime::Controller;
use tugboat_resources::ObjectMetaResource;

pub(crate) struct ReconcilerRunner {
    reconciler: ShipReconciler,
    dependency_tracker: DependencyTracker,
    dependency_rx: mpsc::Receiver<DependencyEvent>,
}

impl ReconcilerRunner {
    pub(crate) fn new(reconciler: ShipReconciler) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let dependency_tracker =
            DependencyTracker::new(reconciler.node_name.clone(), reconciler.client.clone(), tx);
        Self {
            reconciler,
            dependency_tracker,
            dependency_rx: rx,
        }
    }

    pub(crate) async fn run(self) {
        info!(
            "Starting ship reconciliation loop on node '{}'",
            self.reconciler.node_name
        );
        self.reconciler.spawn_watch_shutdown_signal();
        self.reconciler.start_status_collector();

        let ship_controller = Controller::new(self.reconciler.ship_all_api.clone())
            .with_watch_params(
                WatchParams::default()
                    .fields(format!("spec.nodeName={}", self.reconciler.node_name)),
            )
            .with_cancellation_token(self.reconciler.cancellation_token.clone());
        let ship_target_controller = Controller::new(self.reconciler.ship_all_api.clone())
            .with_watch_params(
                WatchParams::default()
                    .fields(format!("spec.targetNodeName={}", self.reconciler.node_name)),
            )
            .with_cancellation_token(self.reconciler.cancellation_token.clone());

        let config_map_controller = self
            .dependency_tracker
            .build_config_map_controller()
            .with_cancellation_token(self.reconciler.cancellation_token.clone());
        let secret_controller = self
            .dependency_tracker
            .build_secret_controller()
            .with_cancellation_token(self.reconciler.cancellation_token.clone());

        let ship_reconciler_fn = {
            let r = self.reconciler.clone();
            move |event| {
                let r = r.clone();
                async move { r.reconcile(event).await }
            }
        };
        let ship_target_reconciler_fn = {
            let r = self.reconciler.clone();
            move |event| {
                let r = r.clone();
                async move { r.reconcile(event).await }
            }
        };

        let config_map_reconciler_fn = {
            let tracker = self.dependency_tracker.clone();
            move |event| {
                let tracker = tracker.clone();
                async move { tracker.handle_config_map_event(event).await }
            }
        };
        let secret_reconciler_fn = {
            let tracker = self.dependency_tracker.clone();
            move |event| {
                let tracker = tracker.clone();
                async move { tracker.handle_secret_event(event).await }
            }
        };

        let cancellation_token = self.reconciler.cancellation_token.clone();
        let dependency_handle = {
            let reconciler = self.reconciler.clone();
            let tracker = self.dependency_tracker.clone();
            let mut rx = self.dependency_rx;
            let token = cancellation_token.clone();
            tokio::spawn(async move {
                loop {
                    select! {
                        Some(event) = rx.recv() => {
                            match event {
                                DependencyEvent::ResourceChanged { kind, namespace, name } => {
                                    if let Err(err) = handle_dependency_change(&reconciler, &tracker, &namespace, kind, &name).await {
                                        warn!("Failed to handle dependency change for {} '{}/{}': {}", kind.as_str(), namespace, name, err);
                                    }
                                }
                            }
                        }
                        _ = token.cancelled() => break,
                    }
                }
            })
        };

        tokio::join!(
            ship_controller.run(ship_reconciler_fn),
            ship_target_controller.run(ship_target_reconciler_fn),
            config_map_controller.run(config_map_reconciler_fn),
            secret_controller.run(secret_reconciler_fn),
            async {
                let _ = dependency_handle.await;
            }
        );
        info!("Ship reconciliation loop stopped.");
    }
}

async fn handle_dependency_change(
    reconciler: &ShipReconciler,
    tracker: &DependencyTracker,
    namespace: &str,
    kind: crate::reconciler::volume::MaterializedVolumeSourceKind,
    name: &str,
) -> Result<(), crate::reconciler::error::ReconcileError> {
    let ships = tracker
        .ships_referencing_resource(namespace, kind, name)
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
        if let Err(err) = reconciler.refresh_materialized_volumes_for_ship(ship).await {
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
