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
use crate::endpoints::resource_handlers;
use crate::endpoints::resource_handlers::{
    ReplaceOptions, ResourceUpdater, validate_resource, validate_resource_name,
};
use crate::endpoints::{ListQuery, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, patch, post, put};
use serde::Deserialize;
use tugboat_resources::ShipMigrationExt;
use tugboat_resources::manifests::core::v1::{Ship, ShipCondition, ShipStatus};
use tugboat_resources::manifests::meta::v1::Time;
use utoipa::ToSchema;

const PHASE_FAILED: &str = "Failed";
const CONDITION_VM_MIGRATION_ABORTED: &str = "VmMigrationAborted";
const MIGRATION_ABORT_MESSAGE: &str = "Live migration aborted via API request";

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = Ship),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
        ),
        request_body = Ship
    )]
#[post("/api/v1/namespaces/{namespace}/ships")]
pub(super) async fn handle_ship_create(
    path: Path<NamespacedPathParams>,
    json: Json<Ship>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, Box<StatusResponse>> {
    resource_handlers::create_namespaced(json.into_inner(), path.into_inner().namespace, operator)
        .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipDeletePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = Ship),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/namespaces/{namespace}/ships/{name}")]
pub(super) async fn handle_ship_delete(
    path: Path<ShipDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Ship>, Box<StatusResponse>> {
    let params = path.into_inner();
    resource_handlers::delete_resource::<Ship>(&operator, Some(params.namespace), params.name).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Ship]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/namespaces/{namespace}/ships")]
pub(super) async fn handle_ship_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Ship>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Ship]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/ships")]
pub(super) async fn handle_ship_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Ship>(&operator, query.into_inner(), None).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipReadPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = Ship),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/namespaces/{namespace}/ships/{name}")]
pub(super) async fn handle_ship_read(
    path: Path<ShipReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Ship>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::read_resource::<Ship>(&operator, Some(path.namespace), path.name).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Ship),
            (status = 400, description = "Invalid targetNodeName", body = StatusResponse),
            (status = 409, description = "Migration is already in progress", body = StatusResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Ship
    )]
#[put("/api/v1/namespaces/{namespace}/ships/{name}")]
pub(super) async fn handle_ship_replace(
    path: Path<ShipReplacePathParams>,
    replacement: Json<Ship>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, Box<StatusResponse>> {
    let path = path.into_inner();
    let namespace = path.namespace;
    let name = path.name;
    let current = get_current_ship(&operator, namespace.clone(), name.clone()).await?;
    let replacement = replacement.into_inner();
    validate_resource_name(&replacement, &name)?;
    let replaced = ResourceUpdater::new(
        &current,
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .apply_replacement(&replacement)?;
    validate_resource(&replaced)?;
    validate_ship_target_node_name_update(&current, &replaced)?;

    let replaced = if current != replaced {
        operator
            .store
            .put(replaced)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .apply_revision()
    } else {
        replaced
    };

    Ok(ModifyResponse::Updated(replaced))
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Ship),
            (status = 400, description = "Invalid targetNodeName", body = StatusResponse),
            (status = 409, description = "Migration is already in progress", body = StatusResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/namespaces/{namespace}/ships/{name}")]
pub(super) async fn handle_ship_patch(
    path: Path<ShipReplacePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, Box<StatusResponse>> {
    let path = path.into_inner();
    let namespace = path.namespace;
    let name = path.name;
    let current = get_current_ship(&operator, namespace.clone(), name.clone()).await?;
    let patched = ResourceUpdater::new(
        &current,
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .apply_patch(patch.into_inner())?;
    validate_resource(&patched)?;
    validate_ship_target_node_name_update(&current, &patched)?;

    let patched = if current != patched {
        operator
            .store
            .put(patched)
            .await
            .map_err(Box::<StatusResponse>::from)?
            .apply_revision()
    } else {
        patched
    };

    Ok(ModifyResponse::Updated(patched))
}

#[utoipa::path(
        responses(
            (status = 200, description = "Ship migration aborted", body = Ship),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 409, description = "Ship is not migrating", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[post("/api/v1/namespaces/{namespace}/ships/{name}/migrate/abort")]
pub(super) async fn handle_ship_migration_abort(
    path: Path<ShipReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, Box<StatusResponse>> {
    let path = path.into_inner();
    let current = operator
        .store
        .get::<Ship>(Some(path.namespace.clone()), &path.name)
        .await
        .map_err(Box::<StatusResponse>::from)?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            "Ship not found",
            Some(serde_json::json!({
                "namespace": path.namespace,
                "name": path.name,
            })),
        )));
    };

    let aborted = abort_ship_migration(current.apply_revision())?;
    let aborted = operator
        .store
        .put(aborted)
        .await
        .map_err(Box::<StatusResponse>::from)?
        .apply_revision();
    Ok(ModifyResponse::Updated(aborted))
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipPatchPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Ship),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/namespaces/{namespace}/ships/{name}/status")]
pub(super) async fn handle_ship_status_patch(
    path: Path<ShipPatchPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::status_patch_resource::<Ship>(
        &operator,
        Some(path.namespace),
        path.name,
        patch.into_inner(),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Ship),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Ship
    )]
#[put("/api/v1/namespaces/{namespace}/ships/{name}/status")]
pub(super) async fn handle_ship_status_replace(
    path: Path<ShipReplacePathParams>,
    replacement: Json<Ship>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::status_replace_resource::<Ship>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
    )
    .await
}

fn abort_ship_migration(mut ship: Ship) -> Result<Ship, Box<StatusResponse>> {
    let target_node_name = ship
        .spec
        .as_ref()
        .and_then(|spec| spec.target_node_name.clone())
        .ok_or_else(|| {
            Box::new(StatusResponse::conflict(
                "Ship does not have a pending migration target",
                None,
            ))
        })?;

    if let Some(spec) = ship.spec.as_mut() {
        spec.target_node_name = None;
    }
    resource_handlers::bump_generation(&mut ship);

    let status = ship.status.get_or_insert_with(ShipStatus::default);
    let mut migration = status.migration.take().unwrap_or_default();
    migration.phase = PHASE_FAILED.to_string();
    migration
        .target_node_name
        .get_or_insert(target_node_name.clone());
    migration.message = MIGRATION_ABORT_MESSAGE.to_string();
    migration.timestamp = Some(Time::now());
    status.migration = Some(migration);
    status.conditions.push(ShipCondition {
        status: CONDITION_VM_MIGRATION_ABORTED.to_string(),
        message: format!(
            "Live migration to node '{}' was aborted by API request",
            target_node_name
        ),
        timestamp: Some(Time::now()),
    });

    Ok(ship)
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

fn validate_ship_target_node_name_update(
    current: &Ship,
    updated: &Ship,
) -> Result<(), Box<StatusResponse>> {
    let updated_spec = updated.spec.as_ref();
    let updated_node_name = updated_spec.and_then(|spec| spec.node_name.as_deref());
    let updated_target_node_name = updated_spec.and_then(|spec| spec.target_node_name.as_deref());

    if matches!(updated_target_node_name, Some("")) {
        return Err(Box::new(StatusResponse::bad_request(
            "spec.targetNodeName must not be empty",
            None,
        )));
    }

    if updated_target_node_name.is_some() && updated_target_node_name == updated_node_name {
        return Err(Box::new(StatusResponse::bad_request(
            "spec.targetNodeName must differ from spec.nodeName",
            None,
        )));
    }

    let current_target_node_name = current
        .spec
        .as_ref()
        .and_then(|spec| spec.target_node_name.as_deref());

    if current_target_node_name != updated_target_node_name && ship_has_active_migration(current) {
        return Err(Box::new(StatusResponse::conflict(
            "Cannot change spec.targetNodeName while migration is in progress; use /migrate/abort to cancel it first",
            None,
        )));
    }

    Ok(())
}

fn ship_has_active_migration(ship: &Ship) -> bool {
    ship.has_active_migration()
}

#[cfg(test)]
mod tests {
    use super::{
        CONDITION_VM_MIGRATION_ABORTED, MIGRATION_ABORT_MESSAGE, PHASE_FAILED,
        abort_ship_migration, ship_has_active_migration, validate_ship_target_node_name_update,
    };
    use actix_web::ResponseError;
    use tugboat_resources::manifests::core::v1::{Ship, ShipMigrationStatus, ShipSpec, ShipStatus};
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    const PHASE_PENDING: &str = "Pending";
    const PHASE_READY: &str = "Ready";
    const PHASE_MIGRATING: &str = "Migrating";

    fn ship_with_target(target: &str) -> Ship {
        Ship {
            spec: Some(ShipSpec {
                target_node_name: Some(target.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn abort_ship_migration_clears_target_and_marks_failed() {
        let ship = Ship {
            spec: Some(ShipSpec {
                target_node_name: Some("node-b".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: "Completed".to_string(),
                    source_node_name: Some("node-a".to_string()),
                    target_node_name: Some("node-b".to_string()),
                    message: "done".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let updated = abort_ship_migration(ship).expect("abort should succeed");

        assert_eq!(
            updated
                .spec
                .as_ref()
                .and_then(|spec| spec.target_node_name.as_deref()),
            None
        );
        assert_eq!(
            updated
                .status
                .as_ref()
                .and_then(|status| status.migration.as_ref())
                .map(|migration| migration.phase.as_str()),
            Some(PHASE_FAILED)
        );
        assert_eq!(
            updated
                .status
                .as_ref()
                .and_then(|status| status.migration.as_ref())
                .map(|migration| migration.message.as_str()),
            Some(MIGRATION_ABORT_MESSAGE)
        );
        assert_eq!(
            updated
                .status
                .as_ref()
                .and_then(|status| status.conditions.last())
                .map(|condition| condition.status.as_str()),
            Some(CONDITION_VM_MIGRATION_ABORTED)
        );
    }

    #[test]
    fn abort_ship_migration_creates_status_when_missing() {
        let updated = abort_ship_migration(ship_with_target("node-b")).expect("abort should work");

        assert!(updated.status.is_some());
        assert_eq!(
            updated
                .status
                .as_ref()
                .and_then(|status| status.migration.as_ref())
                .and_then(|migration| migration.target_node_name.as_deref()),
            Some("node-b")
        );
        assert_eq!(
            updated
                .status
                .as_ref()
                .map(|status| status.conditions.len()),
            Some(1)
        );
    }

    #[test]
    fn abort_ship_migration_bumps_generation() {
        let ship = Ship {
            object_meta: Some(ObjectMeta {
                generation: Some(5),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                target_node_name: Some("node-b".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let updated = abort_ship_migration(ship).expect("abort should succeed");

        assert_eq!(
            updated
                .object_meta
                .as_ref()
                .and_then(|meta| meta.generation),
            Some(6)
        );
    }

    #[test]
    fn abort_ship_migration_rejects_ship_without_target() {
        let err = abort_ship_migration(Ship::default()).expect_err("abort should fail");

        assert_eq!(err.status_code().as_u16(), 409);
    }

    #[test]
    fn validate_target_node_name_rejects_empty_string() {
        let updated = Ship {
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                target_node_name: Some(String::new()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let err = validate_ship_target_node_name_update(&Ship::default(), &updated)
            .expect_err("empty target node name should fail");

        assert_eq!(err.status_code().as_u16(), 400);
    }

    #[test]
    fn validate_target_node_name_rejects_same_as_node_name() {
        let updated = Ship {
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                target_node_name: Some("node-a".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let err = validate_ship_target_node_name_update(&Ship::default(), &updated)
            .expect_err("same target and current node should fail");

        assert_eq!(err.status_code().as_u16(), 400);
    }

    #[test]
    fn validate_target_node_name_rejects_change_during_active_migration() {
        let current = Ship {
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                target_node_name: Some("node-b".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_READY.to_string(),
                    target_node_name: Some("node-b".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let updated = Ship {
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                target_node_name: Some("node-c".to_string()),
                ..Default::default()
            }),
            status: current.status.clone(),
            ..Default::default()
        };

        let err = validate_ship_target_node_name_update(&current, &updated)
            .expect_err("retargeting active migration should fail");

        assert_eq!(err.status_code().as_u16(), 409);
    }

    #[test]
    fn validate_target_node_name_allows_unchanged_target_during_active_migration() {
        let current = Ship {
            spec: Some(ShipSpec {
                node_name: Some("node-a".to_string()),
                target_node_name: Some("node-b".to_string()),
                ..Default::default()
            }),
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_MIGRATING.to_string(),
                    target_node_name: Some("node-b".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        validate_ship_target_node_name_update(&current, &current)
            .expect("unchanged target should be allowed");
    }

    #[test]
    fn ship_has_active_migration_matches_expected_phases() {
        for phase in [PHASE_PENDING, PHASE_READY, PHASE_MIGRATING] {
            let ship = Ship {
                status: Some(ShipStatus {
                    migration: Some(ShipMigrationStatus {
                        phase: phase.to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert!(ship_has_active_migration(&ship), "phase={phase}");
        }

        let ship = Ship {
            status: Some(ShipStatus {
                migration: Some(ShipMigrationStatus {
                    phase: PHASE_FAILED.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!ship_has_active_migration(&ship));
    }
}
