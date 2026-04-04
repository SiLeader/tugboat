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

use crate::data::{ModifyResponse, ReadResponse, StatusResponse};
use crate::endpoints::resource_handlers::ReplaceOptions;
use crate::endpoints::{ListQuery, resource_handlers};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, patch, post, put};
use serde::Serialize;
use tugboat_resources::ShipMigrationExt;
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, NetworkClass, Node, NodeSpec, PersistentVolume, PersistentVolumeClaim,
    Ship, ShipClass,
};
use tugboat_scheduler::framework::{Framework, SchedulingContext};
use tugboat_scheduler::plugins::{create_filter_plugin, create_score_plugin};
use utoipa::ToSchema;

const READ_WRITE_MANY: &str = "ReadWriteMany";
const DRAIN_FILTER_PLUGINS: [&str; 5] = [
    "Unschedulable",
    "NetworkFit",
    "TaintToleration",
    "ResourceFit",
    "StorageFit",
];
const DRAIN_SCORE_PLUGINS: [&str; 2] = ["TaintToleration", "LeastAllocated"];
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeDrainWarning {
    ship: String,
    reason: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeDrainResponse {
    node: Node,
    started_ships: Vec<String>,
    started_count: usize,
    warnings: Vec<NodeDrainWarning>,
}

#[derive(Clone)]
struct DrainPlanningResources {
    nodes: Vec<Node>,
    ships: Vec<Ship>,
    ship_classes: Vec<ShipClass>,
    network_classes: Vec<NetworkClass>,
    cluster_network_classes: Vec<ClusterNetworkClass>,
    persistent_volume_claims: Vec<PersistentVolumeClaim>,
    persistent_volumes: Vec<PersistentVolume>,
}

struct PlannedMigration {
    ship_key: String,
    ship_namespace: String,
    ship_name: String,
    target_node_name: String,
}

struct DrainPlan {
    started: Vec<PlannedMigration>,
    warnings: Vec<NodeDrainWarning>,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = Node),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        request_body = Node
    )]
#[post("/api/v1/nodes")]
pub(super) async fn handle_node_create(
    json: Json<Node>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Node>, Box<StatusResponse>> {
    resource_handlers::create_cluster(json.into_inner(), operator).await
}

#[derive(serde::Deserialize, ToSchema)]
pub(super) struct NodeDeletePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = Node),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/nodes/{name}")]
pub(super) async fn handle_node_delete(
    path: Path<NodeDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Node>, Box<StatusResponse>> {
    resource_handlers::delete_resource::<Node>(&operator, None, path.into_inner().name).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Node]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/nodes")]
pub(super) async fn handle_node_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Node>(&operator, query.into_inner(), None).await
}

#[derive(serde::Deserialize, ToSchema)]
pub(super) struct ReadParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = Node),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/nodes/{name}")]
pub(super) async fn handle_node_read(
    path: Path<ReadParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Node>, Box<StatusResponse>> {
    resource_handlers::read_resource::<Node>(&operator, None, path.into_inner().name).await
}

#[derive(serde::Deserialize, ToSchema)]
pub(super) struct NodeReplacePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Node),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Node
    )]
#[put("/api/v1/nodes/{name}")]
pub(super) async fn handle_node_replace(
    path: Path<NodeReplacePathParams>,
    replacement: Json<Node>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Node>, Box<StatusResponse>> {
    resource_handlers::replace_resource::<Node>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Node),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/nodes/{name}")]
pub(super) async fn handle_node_patch(
    path: Path<NodePatchPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Node>, Box<StatusResponse>> {
    resource_handlers::patch_resource::<Node>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
        },
    )
    .await
}

#[derive(serde::Deserialize, ToSchema)]
pub(super) struct NodePatchPathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = NodeDrainResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[post("/api/v1/nodes/{name}/drain")]
pub(super) async fn handle_node_drain(
    path: Path<NodePatchPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<NodeDrainResponse>, Box<StatusResponse>> {
    let node_name = path.into_inner().name;
    let node = get_current_node(&operator, node_name.clone()).await?;
    let node = mark_node_unschedulable(&operator, node).await?;

    let resources = load_drain_resources(&operator).await?;
    let framework = build_drain_framework();
    let plan = plan_node_drain(&node_name, &resources, &framework);

    let mut started_ships = Vec::with_capacity(plan.started.len());
    for started in &plan.started {
        let current = get_current_ship(
            &operator,
            started.ship_namespace.clone(),
            started.ship_name.clone(),
        )
        .await?;
        let mut updated = current.clone();
        let spec = updated.spec.get_or_insert_with(Default::default);
        spec.target_node_name = Some(started.target_node_name.clone());
        operator
            .store
            .put(updated)
            .await
            .map_err(Box::<StatusResponse>::from)?;
        started_ships.push(started.ship_key.clone());
    }

    Ok(ReadResponse::new(NodeDrainResponse {
        node,
        started_count: started_ships.len(),
        started_ships,
        warnings: plan.warnings,
    }))
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Node),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/nodes/{name}/status")]
pub(super) async fn handle_node_status_patch(
    path: Path<NodePatchPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Node>, Box<StatusResponse>> {
    resource_handlers::status_patch_resource::<Node>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
    )
    .await
}

#[derive(serde::Deserialize, ToSchema)]
pub(super) struct NodeStatusReplacePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Node),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Node
    )]
#[put("/api/v1/nodes/{name}/status")]
pub(super) async fn handle_node_status_replace(
    path: Path<NodeStatusReplacePathParams>,
    replacement: Json<Node>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Node>, Box<StatusResponse>> {
    resource_handlers::status_replace_resource::<Node>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
    )
    .await
}

fn build_drain_framework() -> Framework {
    let mut framework = Framework::new();
    for plugin_name in DRAIN_FILTER_PLUGINS {
        if let Some(plugin) = create_filter_plugin(plugin_name) {
            framework.add_filter_plugin(plugin);
        }
    }
    for plugin_name in DRAIN_SCORE_PLUGINS {
        if let Some(plugin) = create_score_plugin(plugin_name) {
            framework.add_score_plugin(plugin);
        }
    }
    framework
}

fn plan_node_drain(
    node_name: &str,
    resources: &DrainPlanningResources,
    framework: &Framework,
) -> DrainPlan {
    let mut warnings = Vec::new();
    let mut started = Vec::new();
    let mut shadow_ships = resources.ships.clone();

    for ship in &resources.ships {
        let ship_key = ship_key(ship);
        let Some(spec) = ship.spec.as_ref() else {
            continue;
        };

        if spec.node_name.as_deref() != Some(node_name) {
            continue;
        }

        if has_active_migration(ship) {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason: "ship already has an active migration".to_string(),
            });
            continue;
        }

        if spec
            .target_node_name
            .as_deref()
            .is_some_and(|target| !target.is_empty())
        {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason: "ship already has spec.targetNodeName set".to_string(),
            });
            continue;
        }

        if let Err(reason) = ensure_ship_supports_migration_storage(ship, resources) {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason,
            });
            continue;
        }

        let class_name = spec.ship_class.as_str();
        let Some(ship_class) = resources.ship_classes.iter().find(|class| {
            class
                .object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref())
                == Some(class_name)
        }) else {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason: format!("ShipClass '{class_name}' was not found"),
            });
            continue;
        };

        let ctx = SchedulingContext {
            ship: ship.clone(),
            ship_class: ship_class.clone(),
            all_cluster_network_classes: resources.cluster_network_classes.clone(),
            all_network_classes: resources.network_classes.clone(),
            all_ships: shadow_ships.clone(),
            all_ship_classes: resources.ship_classes.clone(),
            all_persistent_volume_claims: resources.persistent_volume_claims.clone(),
            all_persistent_volumes: resources.persistent_volumes.clone(),
        };

        let Some(selected_node) = framework.schedule(&ctx, &resources.nodes) else {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason: "no suitable target node found".to_string(),
            });
            continue;
        };

        let Some(target_node_name) = selected_node
            .object_meta
            .as_ref()
            .and_then(|meta| meta.name.clone())
        else {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason: "selected target node has no metadata.name".to_string(),
            });
            continue;
        };

        if target_node_name == node_name {
            warnings.push(NodeDrainWarning {
                ship: ship_key,
                reason: "selected target node matched the drained node".to_string(),
            });
            continue;
        }

        let ship_namespace = ship
            .object_meta
            .as_ref()
            .and_then(|meta| meta.namespace.clone())
            .unwrap_or_else(|| "default".to_string());
        let ship_name = ship
            .object_meta
            .as_ref()
            .and_then(|meta| meta.name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        started.push(PlannedMigration {
            ship_key: ship_key.clone(),
            ship_namespace,
            ship_name,
            target_node_name: target_node_name.clone(),
        });

        let mut reserved = ship.clone();
        let reserved_spec = reserved.spec.get_or_insert_with(Default::default);
        reserved_spec.node_name = Some(target_node_name);
        reserved_spec.target_node_name = None;
        shadow_ships.push(reserved);
    }

    DrainPlan { started, warnings }
}

fn ensure_ship_supports_migration_storage(
    ship: &Ship,
    resources: &DrainPlanningResources,
) -> Result<(), String> {
    let namespace = ship
        .object_meta
        .as_ref()
        .and_then(|meta| meta.namespace.as_deref())
        .unwrap_or("default");
    let Some(spec) = ship.spec.as_ref() else {
        return Ok(());
    };

    for volume in &spec.volumes {
        let Some(pvc_source) = volume.persistent_volume_claim.as_ref() else {
            continue;
        };

        let Some(pvc) = resources.persistent_volume_claims.iter().find(|pvc| {
            let meta = pvc.object_meta.as_ref();
            meta.and_then(|item| item.name.as_deref()) == Some(pvc_source.claim_name.as_str())
                && meta.and_then(|item| item.namespace.as_deref()) == Some(namespace)
        }) else {
            continue;
        };

        let pvc_name = pvc
            .object_meta
            .as_ref()
            .and_then(|meta| meta.name.as_deref())
            .unwrap_or("<unknown>");
        let pvc_has_rwx = pvc
            .spec
            .as_ref()
            .map(|item| item.access_modes.iter().any(|mode| mode == READ_WRITE_MANY))
            .unwrap_or(false);
        if !pvc_has_rwx {
            return Err(format!(
                "PersistentVolumeClaim '{pvc_name}' does not support '{READ_WRITE_MANY}'"
            ));
        }

        let pv_name = pvc
            .spec
            .as_ref()
            .and_then(|item| item.volume_name.as_deref())
            .unwrap_or("");
        if pv_name.is_empty() {
            continue;
        }

        if let Some(pv) = resources.persistent_volumes.iter().find(|pv| {
            pv.object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref())
                == Some(pv_name)
        }) {
            let pv_has_rwx = pv
                .spec
                .as_ref()
                .map(|item| item.access_modes.iter().any(|mode| mode == READ_WRITE_MANY))
                .unwrap_or(false);
            if !pv_has_rwx {
                return Err(format!(
                    "PersistentVolume '{pv_name}' does not support '{READ_WRITE_MANY}'"
                ));
            }
        }
    }

    Ok(())
}

fn has_active_migration(ship: &Ship) -> bool {
    ship.has_active_migration()
}

fn ship_key(ship: &Ship) -> String {
    let meta = ship.object_meta.as_ref();
    let namespace = meta
        .and_then(|item| item.namespace.as_deref())
        .unwrap_or("default");
    let name = meta
        .and_then(|item| item.name.as_deref())
        .unwrap_or("unknown");
    format!("{namespace}/{name}")
}

async fn load_drain_resources(
    operator: &ApiOperator,
) -> Result<DrainPlanningResources, Box<StatusResponse>> {
    Ok(DrainPlanningResources {
        nodes: operator
            .store
            .list::<Node>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
        ships: operator
            .store
            .list::<Ship>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
        ship_classes: operator
            .store
            .list::<ShipClass>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
        network_classes: operator
            .store
            .list::<NetworkClass>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
        cluster_network_classes: operator
            .store
            .list::<ClusterNetworkClass>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
        persistent_volume_claims: operator
            .store
            .list::<PersistentVolumeClaim>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
        persistent_volumes: operator
            .store
            .list::<PersistentVolume>(None, None)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .into_iter()
            .map(|item| item.apply_revision())
            .collect(),
    })
}

async fn get_current_node(
    operator: &ApiOperator,
    name: String,
) -> Result<Node, Box<StatusResponse>> {
    let current = operator
        .store
        .get::<Node>(None, &name)
        .await
        .map_err(Box::<StatusResponse>::from)?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            "Node not found",
            Some(serde_json::json!({ "name": name })),
        )));
    };
    Ok(current.apply_revision())
}

async fn mark_node_unschedulable(
    operator: &ApiOperator,
    mut node: Node,
) -> Result<Node, Box<StatusResponse>> {
    let spec = node.spec.get_or_insert_with(NodeSpec::default);
    if spec.unschedulable == Some(true) {
        return Ok(node);
    }

    spec.unschedulable = Some(true);
    operator
        .store
        .put(node)
        .await
        .map_err(Box::<StatusResponse>::from)
        .map(|item| item.apply_revision())
}

async fn get_current_ship(
    operator: &ApiOperator,
    namespace: String,
    name: String,
) -> Result<Ship, Box<StatusResponse>> {
    let current = operator
        .store
        .get::<Ship>(Some(namespace.clone()), &name)
        .await
        .map_err(Box::<StatusResponse>::from)?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            "Ship not found",
            Some(serde_json::json!({
                "namespace": namespace,
                "name": name,
            })),
        )));
    };
    Ok(current.apply_revision())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{
        NodeOvercommitSpec, NodeResource, NodeStatus, PersistentVolumeClaimSpec,
        PersistentVolumeClaimVolumeSource, PersistentVolumeSpec, ShipClassSpec,
        ShipMigrationStatus, ShipSpec, ShipVolume,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    fn node(name: &str, cpu: u64, memory: u64, unschedulable: bool) -> Node {
        Node {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            spec: Some(NodeSpec {
                unschedulable: Some(unschedulable),
                resource: Some(NodeResource { cpu, memory }),
                overcommit: Some(NodeOvercommitSpec {
                    cpu_ratio: "1.0".to_string(),
                    memory_ratio: "1.0".to_string(),
                }),
                ..Default::default()
            }),
            status: Some(NodeStatus {
                cni_plugins: vec![
                    tugboat_resources::manifests::core::v1::NodeCniPluginStatus {
                        name: "loopback".to_string(),
                        ready: Some(true),
                        message: "ready".to_string(),
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn ship_class(name: &str, cpu: u64, memory: &str) -> ShipClass {
        ShipClass {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            spec: Some(ShipClassSpec {
                cpu: Some(tugboat_resources::manifests::core::v1::CpuSpec {
                    cores: cpu,
                    ..Default::default()
                }),
                memory: Some(tugboat_resources::manifests::core::v1::MemorySpec {
                    size: memory.to_string(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn ship(name: &str, node_name: &str, ship_class: &str) -> Ship {
        Ship {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some(node_name.to_string()),
                ship_class: ship_class.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn mark_node_unschedulable_sets_field() {
        let node = Node::default();
        let spec = node.spec.unwrap_or_default();
        assert_eq!(spec.unschedulable, None);
    }

    #[test]
    fn drain_plan_starts_schedulable_ship_and_skips_unsupported_ones() {
        let mut active = ship("migrating", "node-a", "small");
        active.status = Some(tugboat_resources::manifests::core::v1::ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: "Migrating".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });

        let pvc = PersistentVolumeClaim {
            object_meta: Some(ObjectMeta {
                name: Some("data".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(PersistentVolumeClaimSpec {
                volume_name: Some("pv-data".to_string()),
                access_modes: vec!["ReadWriteOnce".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let pv = PersistentVolume {
            object_meta: Some(ObjectMeta {
                name: Some("pv-data".to_string()),
                ..Default::default()
            }),
            spec: Some(PersistentVolumeSpec {
                access_modes: vec!["ReadWriteOnce".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let storage_bound = Ship {
            object_meta: Some(ObjectMeta {
                name: Some("rwo".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                ship_class: "small".to_string(),
                volumes: vec![ShipVolume {
                    name: "data".to_string(),
                    persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                        claim_name: "data".to_string(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let resources = DrainPlanningResources {
            nodes: vec![
                node("node-a", 8, 8 * 1024 * 1024 * 1024, true),
                node("node-b", 8, 8 * 1024 * 1024 * 1024, false),
            ],
            ships: vec![ship("ship-a", "node-a", "small"), active, storage_bound],
            ship_classes: vec![ship_class("small", 1, "1Gi")],
            network_classes: Vec::new(),
            cluster_network_classes: Vec::new(),
            persistent_volume_claims: vec![pvc],
            persistent_volumes: vec![pv],
        };

        let plan = plan_node_drain("node-a", &resources, &build_drain_framework());

        assert_eq!(plan.started.len(), 1);
        assert_eq!(plan.started[0].ship_key, "default/ship-a");
        assert_eq!(plan.started[0].target_node_name, "node-b");
        assert_eq!(plan.warnings.len(), 2);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.ship == "default/migrating"
                    && warning.reason.contains("active migration"))
        );
        assert!(plan.warnings.iter().any(
            |warning| warning.ship == "default/rwo" && warning.reason.contains("ReadWriteMany")
        ));
    }
}
