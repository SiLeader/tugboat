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

use crate::cache::Cache;
use crate::config::SchedulerParams;
use crate::framework::Framework;
use crate::framework::SchedulingContext;
use crate::leader_election::LeaderElector;
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::Ship;

pub(crate) struct Scheduler {
    client: TugboatClient,
    framework: Framework,
    leader_elector: LeaderElector,
    config: SchedulerParams,
}

impl Scheduler {
    pub fn new(
        client: TugboatClient,
        framework: Framework,
        leader_elector: LeaderElector,
        config: SchedulerParams,
    ) -> Self {
        Self {
            client,
            framework,
            leader_elector,
            config,
        }
    }

    pub async fn run(mut self) {
        let scheduling_interval =
            std::time::Duration::from_secs(self.config.scheduling_interval_seconds);
        let renew_interval = self.leader_elector.renew_interval();

        tracing::info!(
            "Scheduler '{}' starting, scheduling_interval={}s",
            self.config.name,
            self.config.scheduling_interval_seconds,
        );

        let mut cache = Cache::new(self.client.clone());

        loop {
            // Leader election: try to acquire or renew
            self.leader_elector.try_acquire_or_renew().await;

            if !self.leader_elector.is_leader() {
                tracing::debug!("Not the leader, waiting...");
                tokio::time::sleep(renew_interval).await;
                continue;
            }

            // Refresh cache
            if let Err(e) = cache.refresh().await {
                tracing::error!("Failed to refresh cache: {e}");
                tokio::time::sleep(scheduling_interval).await;
                continue;
            }

            // Find unscheduled Ships
            let unscheduled: Vec<&Ship> = cache
                .ships()
                .iter()
                .filter(|ship| {
                    let spec = ship.spec.as_ref();
                    let node_name = spec.and_then(|s| s.node_name.as_deref());
                    let scheduler_name = spec.and_then(|s| s.scheduler_name.as_deref());

                    // Ship must have no node assigned
                    node_name.is_none_or(|n| n.is_empty())
                        // Ship must be for this scheduler (or unspecified)
                        && scheduler_name
                            .is_none_or(|n| n.is_empty() || n == self.config.name)
                })
                .collect();

            if !unscheduled.is_empty() {
                tracing::info!("Found {} unscheduled ship(s)", unscheduled.len());
            }

            for ship in unscheduled {
                self.schedule_ship(ship, &cache).await;
            }

            tokio::time::sleep(scheduling_interval).await;
        }
    }

    async fn schedule_ship(&self, ship: &Ship, cache: &Cache) {
        let ship_name = ship
            .object_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            .unwrap_or("unknown");
        let ship_namespace = ship
            .object_meta
            .as_ref()
            .and_then(|m| m.namespace.as_deref())
            .unwrap_or("default");

        let class_name = ship
            .spec
            .as_ref()
            .map(|s| s.ship_class.as_str())
            .unwrap_or("");
        let requested_runtime_class = ship
            .spec
            .as_ref()
            .and_then(|spec| spec.runtime_class.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let Some(ship_class) = cache.find_ship_class(class_name) else {
            tracing::warn!(
                "ShipClass '{class_name}' not found for ship {ship_namespace}/{ship_name}"
            );
            return;
        };

        if let Some(runtime_class_name) = requested_runtime_class
            && cache.find_runtime_class(runtime_class_name).is_none()
        {
            tracing::warn!(
                "RuntimeClass '{runtime_class_name}' not found for ship {ship_namespace}/{ship_name}"
            );
            return;
        }

        let ctx = SchedulingContext {
            ship: ship.clone(),
            ship_class: ship_class.clone(),
            all_cluster_network_classes: cache.cluster_network_classes().to_vec(),
            all_network_classes: cache.network_classes().to_vec(),
            all_runtime_classes: cache.runtime_classes().to_vec(),
            all_ships: cache.ships().to_vec(),
            all_ship_classes: cache.ship_classes().to_vec(),
            all_persistent_volume_claims: cache.persistent_volume_claims().to_vec(),
            all_persistent_volumes: cache.persistent_volumes().to_vec(),
        };

        let Some(selected_node) = self.framework.schedule(&ctx, cache.nodes()) else {
            tracing::warn!("No suitable node found for ship {ship_namespace}/{ship_name}");
            return;
        };

        let node_name = selected_node
            .object_meta
            .as_ref()
            .and_then(|m| m.name.as_deref())
            .unwrap_or("unknown");

        tracing::info!("Scheduling ship {ship_namespace}/{ship_name} to node {node_name}");

        // Bind: update the Ship's spec.nodeName
        let mut updated = ship.clone();
        if let Some(ref mut spec) = updated.spec {
            spec.node_name = Some(node_name.to_string());
        }

        let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), ship_namespace);
        match ship_api.replace(ship_name, updated).await {
            Ok(_) => {
                tracing::info!(
                    "Successfully bound ship {ship_namespace}/{ship_name} to node {node_name}"
                );
            }
            Err(e) => {
                tracing::error!(
                    "Failed to bind ship {ship_namespace}/{ship_name} to node {node_name}: {e}"
                );
            }
        }
    }
}
