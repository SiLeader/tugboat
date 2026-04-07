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
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipStatus};
use tugboat_resources::manifests::meta::v1::Time;

const CONDITION_SCHEDULING_BLOCKED: &str = "SchedulingBlocked";

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
            let message = format!("ShipClass '{class_name}' not found");
            tracing::warn!("{message} for ship {ship_namespace}/{ship_name}");
            self.update_ship_scheduling_status(
                ship,
                ship_namespace,
                ship_name,
                CONDITION_SCHEDULING_BLOCKED,
                message,
            )
            .await;
            return;
        };

        if let Some(runtime_class_name) = requested_runtime_class
            && cache.find_runtime_class(runtime_class_name).is_none()
        {
            let message = format!("RuntimeClass '{runtime_class_name}' not found");
            tracing::warn!("{message} for ship {ship_namespace}/{ship_name}");
            self.update_ship_scheduling_status(
                ship,
                ship_namespace,
                ship_name,
                CONDITION_SCHEDULING_BLOCKED,
                message,
            )
            .await;
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

    async fn update_ship_scheduling_status(
        &self,
        ship: &Ship,
        ship_namespace: &str,
        ship_name: &str,
        status: &str,
        message: String,
    ) {
        let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), ship_namespace);
        let mut status_ship = ship.clone();
        append_ship_condition(
            &mut status_ship,
            ShipCondition {
                status: status.to_string(),
                message,
                timestamp: Some(Time::now()),
            },
        );

        if let Err(error) = ship_api.replace_status(ship_name, status_ship).await {
            tracing::error!(
                "Failed to update scheduling status for ship {ship_namespace}/{ship_name}: {error}"
            );
        }
    }
}

fn append_ship_condition(ship: &mut Ship, condition: ShipCondition) {
    let status = ship.status.get_or_insert_with(ShipStatus::default);
    upsert_ship_condition(&mut status.conditions, condition);
}

fn upsert_ship_condition(conditions: &mut Vec<ShipCondition>, condition: ShipCondition) {
    if let Some(existing) = conditions
        .iter_mut()
        .find(|item| item.status == condition.status)
    {
        if existing.message == condition.message {
            return;
        }
        *existing = condition;
    } else {
        conditions.push(condition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_ship_condition_adds_new_condition_when_missing() {
        let mut ship = Ship::default();

        append_ship_condition(
            &mut ship,
            ShipCondition {
                status: CONDITION_SCHEDULING_BLOCKED.to_string(),
                message: "RuntimeClass 'kata' not found".to_string(),
                timestamp: None,
            },
        );

        let status = ship.status.expect("ship status should be initialized");
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(status.conditions[0].status, CONDITION_SCHEDULING_BLOCKED);
        assert_eq!(
            status.conditions[0].message,
            "RuntimeClass 'kata' not found"
        );
    }

    #[test]
    fn append_ship_condition_updates_existing_condition_with_same_status() {
        let mut ship = Ship {
            status: Some(ShipStatus {
                conditions: vec![ShipCondition {
                    status: CONDITION_SCHEDULING_BLOCKED.to_string(),
                    message: "ShipClass 'small' not found".to_string(),
                    timestamp: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        append_ship_condition(
            &mut ship,
            ShipCondition {
                status: CONDITION_SCHEDULING_BLOCKED.to_string(),
                message: "RuntimeClass 'kata' not found".to_string(),
                timestamp: None,
            },
        );

        let status = ship.status.expect("ship status should exist");
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(
            status.conditions[0].message,
            "RuntimeClass 'kata' not found"
        );
    }
}
