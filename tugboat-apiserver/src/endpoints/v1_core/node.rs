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
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Node;
use utoipa::ToSchema;

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

#[derive(Deserialize, ToSchema)]
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

#[derive(Deserialize, ToSchema)]
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

#[derive(Deserialize, ToSchema)]
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

#[derive(Deserialize, ToSchema)]
pub(super) struct NodePatchPathParams {
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

#[derive(Deserialize, ToSchema)]
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
