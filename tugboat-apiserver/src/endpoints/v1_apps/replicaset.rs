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
use tugboat_resources::manifests::apps::v1::ReplicaSet;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = ReplicaSet),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
        ),
        request_body = ReplicaSet
    )]
#[post("/apis/apps/v1/namespaces/{namespace}/replicasets")]
pub(super) async fn handle_replicaset_create(
    path: Path<NamespacedPathParams>,
    json: Json<ReplicaSet>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ReplicaSet>, Box<StatusResponse>> {
    resource_handlers::create_namespaced(json.into_inner(), path.into_inner().namespace, operator)
        .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ReplicaSetPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = ReplicaSet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/apis/apps/v1/namespaces/{namespace}/replicasets/{name}")]
pub(super) async fn handle_replicaset_delete(
    path: Path<ReplicaSetPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ReplicaSet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::delete_resource::<ReplicaSet>(&operator, Some(path.namespace), path.name)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [ReplicaSet]),
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
#[get("/apis/apps/v1/namespaces/{namespace}/replicasets")]
pub(super) async fn handle_replicaset_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<ReplicaSet>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [ReplicaSet]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/apis/apps/v1/replicasets")]
pub(super) async fn handle_replicaset_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<ReplicaSet>(&operator, query.into_inner(), None).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = ReplicaSet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/apis/apps/v1/namespaces/{namespace}/replicasets/{name}")]
pub(super) async fn handle_replicaset_read(
    path: Path<ReplicaSetPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ReplicaSet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::read_resource::<ReplicaSet>(&operator, Some(path.namespace), path.name).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = ReplicaSet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = ReplicaSet
    )]
#[put("/apis/apps/v1/namespaces/{namespace}/replicasets/{name}")]
pub(super) async fn handle_replicaset_replace(
    path: Path<ReplicaSetPathParams>,
    replacement: Json<ReplicaSet>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ReplicaSet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::replace_resource::<ReplicaSet>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = ReplicaSet),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/apis/apps/v1/namespaces/{namespace}/replicasets/{name}")]
pub(super) async fn handle_replicaset_patch(
    path: Path<ReplicaSetPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ReplicaSet>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::patch_resource::<ReplicaSet>(
        &operator,
        Some(path.namespace),
        path.name,
        patch.into_inner(),
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
        },
    )
    .await
}
