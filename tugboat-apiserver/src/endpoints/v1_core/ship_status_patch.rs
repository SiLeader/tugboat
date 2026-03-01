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

use crate::data::{ModifyResponse, StatusResponse};
use crate::endpoints::resource_handlers;
use crate::operator::ApiOperator;
use actix_web::patch;
use actix_web::web::{Data, Json, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Ship;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipPatchPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path()]
#[patch("/api/v1/namespaces/{namespace}/ships/{name}/status")]
pub(super) async fn handle_ship_status_patch(
    path: Path<ShipPatchPathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, StatusResponse> {
    let path = path.into_inner();
    resource_handlers::status_patch_resource::<Ship>(
        &operator,
        Some(path.namespace),
        path.name,
        patch.into_inner(),
    )
    .await
}
