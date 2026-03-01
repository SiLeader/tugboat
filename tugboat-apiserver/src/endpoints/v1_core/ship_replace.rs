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
use crate::endpoints::resource_handlers::{self, ReplaceOptions};
use crate::operator::ApiOperator;
use actix_web::put;
use actix_web::web::{Data, Json, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Ship;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path()]
#[put("/api/v1/namespaces/{namespace}/ships/{name}")]
pub(super) async fn handle_ship_replace(
    path: Path<ShipReplacePathParams>,
    replacement: Json<Ship>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, StatusResponse> {
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
