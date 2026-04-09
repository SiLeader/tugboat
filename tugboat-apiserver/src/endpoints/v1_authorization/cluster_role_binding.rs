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
use tugboat_resources::manifests::authorization::v1::ClusterRoleBinding;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = ClusterRoleBinding),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        request_body = ClusterRoleBinding
    )]
#[post("/apis/authorization/v1/clusterrolebindings")]
pub(super) async fn handle_cluster_role_binding_create(
    json: Json<ClusterRoleBinding>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ClusterRoleBinding>, Box<StatusResponse>> {
    resource_handlers::create_cluster(json.into_inner(), operator).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ClusterRoleBindingPathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = ClusterRoleBinding),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/apis/authorization/v1/clusterrolebindings/{name}")]
pub(super) async fn handle_cluster_role_binding_delete(
    path: Path<ClusterRoleBindingPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ClusterRoleBinding>, Box<StatusResponse>> {
    resource_handlers::delete_resource::<ClusterRoleBinding>(
        &operator,
        None,
        path.into_inner().name,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [ClusterRoleBinding]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/apis/authorization/v1/clusterrolebindings")]
pub(super) async fn handle_cluster_role_binding_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<ClusterRoleBinding>(&operator, query.into_inner(), None)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = ClusterRoleBinding),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/apis/authorization/v1/clusterrolebindings/{name}")]
pub(super) async fn handle_cluster_role_binding_read(
    path: Path<ClusterRoleBindingPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ClusterRoleBinding>, Box<StatusResponse>> {
    resource_handlers::read_resource::<ClusterRoleBinding>(&operator, None, path.into_inner().name)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = ClusterRoleBinding),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = ClusterRoleBinding
    )]
#[put("/apis/authorization/v1/clusterrolebindings/{name}")]
pub(super) async fn handle_cluster_role_binding_replace(
    path: Path<ClusterRoleBindingPathParams>,
    replacement: Json<ClusterRoleBinding>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ClusterRoleBinding>, Box<StatusResponse>> {
    resource_handlers::replace_resource::<ClusterRoleBinding>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = ClusterRoleBinding),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/apis/authorization/v1/clusterrolebindings/{name}")]
pub(super) async fn handle_cluster_role_binding_patch(
    path: Path<ClusterRoleBindingPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ClusterRoleBinding>, Box<StatusResponse>> {
    resource_handlers::patch_resource::<ClusterRoleBinding>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .await
}
