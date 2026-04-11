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
use crate::endpoints::resource_handlers::ReplaceOptions;
use crate::endpoints::{ListQuery, NamespacedNamePathParams, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, patch, post, put};
use tugboat_resources::manifests::apps::v1::Fleet;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = Fleet),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
        ),
        request_body = Fleet
    )]
#[post("/apis/apps/v1/namespaces/{namespace}/fleets")]
pub(super) async fn handle_fleet_create(
    path: Path<NamespacedPathParams>,
    json: Json<Fleet>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Fleet>, Box<StatusResponse>> {
    resource_handlers::create_namespaced(json.into_inner(), path.into_inner().namespace, operator)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = Fleet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/apis/apps/v1/namespaces/{namespace}/fleets/{name}")]
pub(super) async fn handle_fleet_delete(
    path: Path<NamespacedNamePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Fleet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::delete_resource::<Fleet>(&operator, Some(path.namespace), path.name).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Fleet]),
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
#[get("/apis/apps/v1/namespaces/{namespace}/fleets")]
pub(super) async fn handle_fleet_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Fleet>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Fleet]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/apis/apps/v1/fleets")]
pub(super) async fn handle_fleet_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Fleet>(&operator, query.into_inner(), None).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = Fleet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/apis/apps/v1/namespaces/{namespace}/fleets/{name}")]
pub(super) async fn handle_fleet_read(
    path: Path<NamespacedNamePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Fleet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::read_resource::<Fleet>(&operator, Some(path.namespace), path.name).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Fleet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Fleet
    )]
#[put("/apis/apps/v1/namespaces/{namespace}/fleets/{name}")]
pub(super) async fn handle_fleet_replace(
    path: Path<NamespacedNamePathParams>,
    replacement: Json<Fleet>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Fleet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::replace_resource::<Fleet>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Fleet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/apis/apps/v1/namespaces/{namespace}/fleets/{name}")]
pub(super) async fn handle_fleet_patch(
    path: Path<NamespacedNamePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Fleet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::patch_resource::<Fleet>(
        &operator,
        Some(path.namespace),
        path.name,
        patch.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Fleet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/apis/apps/v1/namespaces/{namespace}/fleets/{name}/status")]
pub(super) async fn handle_fleet_status_patch(
    path: Path<NamespacedNamePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Fleet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::status_patch_resource::<Fleet>(
        &operator,
        Some(path.namespace),
        path.name,
        patch.into_inner(),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Fleet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Fleet
    )]
#[put("/apis/apps/v1/namespaces/{namespace}/fleets/{name}/status")]
pub(super) async fn handle_fleet_status_replace(
    path: Path<NamespacedNamePathParams>,
    replacement: Json<Fleet>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Fleet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::status_replace_resource::<Fleet>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
    )
    .await
}
