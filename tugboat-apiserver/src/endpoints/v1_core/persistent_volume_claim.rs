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
use tugboat_resources::manifests::core::v1::PersistentVolumeClaim;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = PersistentVolumeClaim),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
        ),
        request_body = PersistentVolumeClaim
    )]
#[post("/api/v1/namespaces/{namespace}/persistentvolumeclaims")]
pub(super) async fn handle_persistent_volume_claim_create(
    path: Path<NamespacedPathParams>,
    json: Json<PersistentVolumeClaim>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolumeClaim>, Box<StatusResponse>> {
    resource_handlers::create_namespaced(json.into_inner(), path.into_inner().namespace, operator)
        .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeClaimDeletePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = PersistentVolumeClaim),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/namespaces/{namespace}/persistentvolumeclaims/{name}")]
pub(super) async fn handle_persistent_volume_claim_delete(
    path: Path<PersistentVolumeClaimDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<PersistentVolumeClaim>, Box<StatusResponse>> {
    let params = path.into_inner();
    resource_handlers::delete_resource::<PersistentVolumeClaim>(
        &operator,
        Some(params.namespace),
        params.name,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [PersistentVolumeClaim]),
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
#[get("/api/v1/namespaces/{namespace}/persistentvolumeclaims")]
pub(super) async fn handle_persistent_volume_claim_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<PersistentVolumeClaim>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [PersistentVolumeClaim]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/persistentvolumeclaims")]
pub(super) async fn handle_persistent_volume_claim_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<PersistentVolumeClaim>(&operator, query.into_inner(), None)
        .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeClaimReadPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = PersistentVolumeClaim),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/namespaces/{namespace}/persistentvolumeclaims/{name}")]
pub(super) async fn handle_persistent_volume_claim_read(
    path: Path<PersistentVolumeClaimReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<PersistentVolumeClaim>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::read_resource::<PersistentVolumeClaim>(
        &operator,
        Some(path.namespace),
        path.name,
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeClaimReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = PersistentVolumeClaim),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = PersistentVolumeClaim
    )]
#[put("/api/v1/namespaces/{namespace}/persistentvolumeclaims/{name}")]
pub(super) async fn handle_persistent_volume_claim_replace(
    path: Path<PersistentVolumeClaimReplacePathParams>,
    replacement: Json<PersistentVolumeClaim>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolumeClaim>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::replace_resource::<PersistentVolumeClaim>(
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
pub(super) struct PersistentVolumeClaimPatchPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = PersistentVolumeClaim),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/namespaces/{namespace}/persistentvolumeclaims/{name}/status")]
pub(super) async fn handle_persistent_volume_claim_status_patch(
    path: Path<PersistentVolumeClaimPatchPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolumeClaim>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::status_patch_resource::<PersistentVolumeClaim>(
        &operator,
        Some(path.namespace),
        path.name,
        patch.into_inner(),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeClaimStatusReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = PersistentVolumeClaim),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = PersistentVolumeClaim
    )]
#[put("/api/v1/namespaces/{namespace}/persistentvolumeclaims/{name}/status")]
pub(super) async fn handle_persistent_volume_claim_status_replace(
    path: Path<PersistentVolumeClaimStatusReplacePathParams>,
    replacement: Json<PersistentVolumeClaim>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolumeClaim>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::status_replace_resource::<PersistentVolumeClaim>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
    )
    .await
}
