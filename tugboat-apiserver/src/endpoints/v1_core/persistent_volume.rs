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
use tugboat_resources::manifests::core::v1::PersistentVolume;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = PersistentVolume),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        request_body = PersistentVolume
    )]
#[post("/api/v1/persistentvolumes")]
pub(super) async fn handle_persistent_volume_create(
    json: Json<PersistentVolume>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolume>, StatusResponse> {
    resource_handlers::create_cluster(json.into_inner(), operator).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeDeletePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = PersistentVolume),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/persistentvolumes/{name}")]
pub(super) async fn handle_persistent_volume_delete(
    path: Path<PersistentVolumeDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<PersistentVolume>, StatusResponse> {
    resource_handlers::delete_resource::<PersistentVolume>(&operator, None, path.into_inner().name)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [PersistentVolume]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/persistentvolumes")]
pub(super) async fn handle_persistent_volume_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    resource_handlers::list_resources::<PersistentVolume>(&operator, query.into_inner(), None).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeReadPathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = PersistentVolume),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/persistentvolumes/{name}")]
pub(super) async fn handle_persistent_volume_read(
    path: Path<PersistentVolumeReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<PersistentVolume>, StatusResponse> {
    resource_handlers::read_resource::<PersistentVolume>(&operator, None, path.into_inner().name)
        .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeReplacePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = PersistentVolume),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = PersistentVolume
    )]
#[put("/api/v1/persistentvolumes/{name}")]
pub(super) async fn handle_persistent_volume_replace(
    path: Path<PersistentVolumeReplacePathParams>,
    replacement: Json<PersistentVolume>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolume>, StatusResponse> {
    resource_handlers::replace_resource::<PersistentVolume>(
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
pub(super) struct PersistentVolumePatchPathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = PersistentVolume),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/persistentvolumes/{name}/status")]
pub(super) async fn handle_persistent_volume_status_patch(
    path: Path<PersistentVolumePatchPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolume>, StatusResponse> {
    resource_handlers::status_patch_resource::<PersistentVolume>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct PersistentVolumeStatusReplacePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = PersistentVolume),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = PersistentVolume
    )]
#[put("/api/v1/persistentvolumes/{name}/status")]
pub(super) async fn handle_persistent_volume_status_replace(
    path: Path<PersistentVolumeStatusReplacePathParams>,
    replacement: Json<PersistentVolume>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<PersistentVolume>, StatusResponse> {
    resource_handlers::status_replace_resource::<PersistentVolume>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
    )
    .await
}
