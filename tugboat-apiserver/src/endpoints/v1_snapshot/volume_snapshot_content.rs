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
use crate::endpoints::{ClusterNamePathParams, ListQuery, resource_handlers};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, patch, post, put};
use tugboat_resources::manifests::snapshot::v1::VolumeSnapshotContent;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = VolumeSnapshotContent),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        request_body = VolumeSnapshotContent
    )]
#[post("/apis/snapshot/v1/volumesnapshotcontents")]
pub(super) async fn handle_volume_snapshot_content_create(
    json: Json<VolumeSnapshotContent>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::create_cluster(json.into_inner(), operator).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = VolumeSnapshotContent),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource"))
    )]
#[delete("/apis/snapshot/v1/volumesnapshotcontents/{name}")]
pub(super) async fn handle_volume_snapshot_content_delete(
    path: Path<ClusterNamePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::delete_resource::<VolumeSnapshotContent>(
        &operator,
        None,
        path.into_inner().name,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [VolumeSnapshotContent]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/apis/snapshot/v1/volumesnapshotcontents")]
pub(super) async fn handle_volume_snapshot_content_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<VolumeSnapshotContent>(&operator, query.into_inner(), None)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = VolumeSnapshotContent),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource"))
    )]
#[get("/apis/snapshot/v1/volumesnapshotcontents/{name}")]
pub(super) async fn handle_volume_snapshot_content_read(
    path: Path<ClusterNamePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::read_resource::<VolumeSnapshotContent>(
        &operator,
        None,
        path.into_inner().name,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = VolumeSnapshotContent),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = VolumeSnapshotContent
    )]
#[put("/apis/snapshot/v1/volumesnapshotcontents/{name}")]
pub(super) async fn handle_volume_snapshot_content_replace(
    path: Path<ClusterNamePathParams>,
    replacement: Json<VolumeSnapshotContent>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::replace_resource::<VolumeSnapshotContent>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = VolumeSnapshotContent),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = Object
    )]
#[patch("/apis/snapshot/v1/volumesnapshotcontents/{name}")]
pub(super) async fn handle_volume_snapshot_content_patch(
    path: Path<ClusterNamePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::patch_resource::<VolumeSnapshotContent>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = VolumeSnapshotContent),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = Object
    )]
#[patch("/apis/snapshot/v1/volumesnapshotcontents/{name}/status")]
pub(super) async fn handle_volume_snapshot_content_status_patch(
    path: Path<ClusterNamePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::status_patch_resource::<VolumeSnapshotContent>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = VolumeSnapshotContent),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = VolumeSnapshotContent
    )]
#[put("/apis/snapshot/v1/volumesnapshotcontents/{name}/status")]
pub(super) async fn handle_volume_snapshot_content_status_replace(
    path: Path<ClusterNamePathParams>,
    replacement: Json<VolumeSnapshotContent>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<VolumeSnapshotContent>, Box<StatusResponse>> {
    resource_handlers::status_replace_resource::<VolumeSnapshotContent>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
    )
    .await
}
