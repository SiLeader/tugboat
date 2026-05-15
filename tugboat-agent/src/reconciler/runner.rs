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
use crate::reconciler::dependency::{DependencyChangeKind, DependencyEvent, DependencyTracker};
use crate::reconciler::ops::snapshot::{AgentSnapshotContext, SnapshotStateMachine};
use tokio::select;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use tugboat_client::Api;
use tugboat_client::WatchParams;
use tugboat_client::runtime::{Action, Controller, FinalizerEvent, ReconcileEvent, finalizer};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{Ship, ShipSnapshot};

const SHIP_SNAPSHOT_AGENT_FINALIZER_PREFIX: &str = "snapshot.tugboat.cloud/agent-";

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
        let ship_snapshot_controller = Controller::new(tugboat_client::Api::<ShipSnapshot>::all(
            self.reconciler.client.clone(),
        ))
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
        let snapshot_reconciler_fn = {
            let node_name = self.reconciler.node_name.clone();
            let client = self.reconciler.client.clone();
            let runtime_operator = self.reconciler.runtime_operator.clone();
            move |event: ReconcileEvent<ShipSnapshot>| {
                let context = AgentSnapshotContext {
                    node_name: node_name.clone(),
                    client: client.clone(),
                    runtime_operator: runtime_operator.clone(),
                };
                async move { reconcile_ship_snapshot(context, event).await }
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
                                DependencyEvent::ResourceChanged { kind, namespace, name, change } => {
                                    if let Err(err) = handle_dependency_change(&reconciler, &tracker, &namespace, kind, &name, change).await {
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
            ship_snapshot_controller.run(snapshot_reconciler_fn),
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
    change: DependencyChangeKind,
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
        if change == DependencyChangeKind::Deleted
            && let Err(err) =
                reconciler.clear_materialized_volumes_for_dependency(&ship, kind, name)
        {
            warn!(
                "Failed to clear stale materialized volumes for Ship '{}/{}' after {} '{}/{}' was deleted: {}",
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
            continue;
        }
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

async fn reconcile_ship_snapshot(
    context: AgentSnapshotContext,
    event: ReconcileEvent<ShipSnapshot>,
) -> Result<Action, crate::reconciler::error::ReconcileError> {
    match event {
        ReconcileEvent::Applied(snapshot) => {
            if snapshot.deletion_timestamp().is_none()
                && !ship_snapshot_targets_node(&context, &snapshot).await?
            {
                let machine = SnapshotStateMachine::new(&context);
                machine.reconcile(&snapshot).await?;
                return Ok(Action::await_change());
            }
            let namespace = snapshot.namespace().unwrap_or("default");
            let api: Api<ShipSnapshot> = Api::namespaced(context.client.clone(), namespace);
            let finalizer_name = ship_snapshot_agent_finalizer(&context.node_name);
            finalizer::<ShipSnapshot, crate::reconciler::error::ReconcileError, _, _>(
                &api,
                &finalizer_name,
                snapshot,
                {
                    let context = context.clone();
                    move |event| async move {
                        let machine = SnapshotStateMachine::new(&context);
                        match event {
                            FinalizerEvent::Apply(snapshot) => {
                                machine.reconcile(&snapshot).await?;
                                Ok(Action::await_change())
                            }
                            FinalizerEvent::Cleanup(snapshot) => {
                                machine.cleanup(&snapshot).await?;
                                Ok(Action::await_change())
                            }
                        }
                    }
                },
            )
            .await
            .map_err(|err| crate::reconciler::error::ReconcileError::Finalizer(err.to_string()))
        }
        ReconcileEvent::Deleted(snapshot) => {
            let machine = SnapshotStateMachine::new(&context);
            if let Err(err) = machine.cleanup(&snapshot).await {
                warn!(
                    "Best-effort ShipSnapshot cleanup failed on node '{}': {}",
                    context.node_name, err
                );
            }
            Ok(Action::await_change())
        }
    }
}

async fn ship_snapshot_targets_node(
    context: &AgentSnapshotContext,
    snapshot: &ShipSnapshot,
) -> Result<bool, crate::reconciler::error::ReconcileError> {
    let Some(spec) = snapshot.spec.as_ref() else {
        return Ok(false);
    };
    let ship_name = spec.ship_name.trim();
    if ship_name.is_empty() {
        return Ok(false);
    }
    let Some(namespace) = snapshot.namespace() else {
        return Ok(false);
    };
    let ship_api: Api<Ship> = Api::namespaced(context.client.clone(), namespace);
    let Some(ship) = ship_api.get(ship_name).await? else {
        return Ok(false);
    };
    Ok(ship
        .spec
        .as_ref()
        .and_then(|spec| spec.node_name.as_deref())
        == Some(context.node_name.as_str()))
}

fn ship_snapshot_agent_finalizer(node_name: &str) -> String {
    format!("{SHIP_SNAPSHOT_AGENT_FINALIZER_PREFIX}{node_name}")
}
