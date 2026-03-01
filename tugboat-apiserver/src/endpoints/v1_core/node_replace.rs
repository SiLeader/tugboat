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

#[utoipa::path()]
#[put("/api/v1/nodes/{name}")]
pub(super) async fn handle_node_replace(
    path: Path<NodeReplacePathParams>,
    replacement: Json<Node>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Node>, StatusResponse> {
    let path = path.into_inner();
    let current = operator.store.get::<Node>(None, &path.name).await?;
    let Some(current) = current else {
        return Err(StatusResponse::not_found(
            "Node not found.",
            Some(serde_json::json!({"name": path.name})),
        ));
    };
    let current = current.apply_revision();
    let replacement = replacement.into_inner();

    let mut replaced_meta = current.object_meta.clone().unwrap_or_default();
    if let Some(client_rv) = replacement
        .object_meta
        .as_ref()
        .and_then(|m| m.resource_version.clone())
    {
        replaced_meta.resource_version = Some(client_rv);
    }

    let replaced = Node {
        object_meta: Some(replaced_meta),
        type_meta: current.type_meta.clone(),
        spec: replacement.spec,
    };

    let replaced = if current != replaced {
        operator.store.put(replaced).await?.apply_revision()
    } else {
        replaced
    };
    Ok(ModifyResponse::Updated(replaced))
}
