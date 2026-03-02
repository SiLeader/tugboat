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
use tugboat_resources::manifests::core::v1::Node;
use utoipa::ToSchema;

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
) -> Result<ModifyResponse<Node>, StatusResponse> {
    resource_handlers::replace_resource::<Node>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
        },
    )
    .await
}
