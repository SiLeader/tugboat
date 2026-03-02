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
use actix_web::get;
use actix_web::web::{Data, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::ClusterNetworkClass;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct ReadParams {
    name: String,
}

#[utoipa::path(
    responses(
        (status = 200, description = "Resource details", body = ClusterNetworkClass),
        (status = 404, description = "Resource not found", body = StatusResponse),
        (status = 500, description = "Internal server error", body = StatusResponse),
    ),
    params(
        ("name" = String, Path, description = "Name of the resource"),
    )
)]
#[get("/api/v1/clusternetworkclasses/{name}")]
pub(super) async fn handle_clusternetworkclass_read(
    path: Path<ReadParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ClusterNetworkClass>, StatusResponse> {
    resource_handlers::read_resource::<ClusterNetworkClass>(&operator, None, path.into_inner().name)
        .await
}
