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

use crate::data::{ReadResponse, StatusResponse};
use crate::endpoints::resource_handlers;
use crate::operator::ApiOperator;
use actix_web::delete;
use actix_web::web::{Data, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::PersistentVolumeClaim;
use utoipa::ToSchema;

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
) -> Result<ReadResponse<PersistentVolumeClaim>, StatusResponse> {
    let params = path.into_inner();
    resource_handlers::delete_resource::<PersistentVolumeClaim>(
        &operator,
        Some(params.namespace),
        params.name,
    )
    .await
}
