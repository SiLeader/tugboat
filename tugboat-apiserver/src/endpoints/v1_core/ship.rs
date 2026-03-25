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
use crate::endpoints::{ListQuery, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, patch, post, put};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Ship;
use utoipa::ToSchema;

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
    resource_handlers::replace_resource::<Ship>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
        },
    )
    .await
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
