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
use tugboat_resources::manifests::coordination::v1::Lease;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct LeaseReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path()]
#[put("/apis/coordination/v1/namespaces/{namespace}/leases/{name}")]
pub(super) async fn handle_lease_replace(
    path: Path<LeaseReplacePathParams>,
    replacement: Json<Lease>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Lease>, StatusResponse> {
    let path = path.into_inner();
    let current = operator
        .store
        .get::<Lease>(Some(path.namespace.clone()), &path.name)
        .await?;
    let Some(current) = current else {
        return Err(StatusResponse::not_found(
            "Lease not found.",
            Some(serde_json::json!({"name": path.name, "namespace": path.namespace})),
        ));
    };
    let current = current.apply_revision();
    let replacement = replacement.into_inner();

    // Use the client's resource_version for optimistic concurrency control.
    // The store's compare-and-swap will reject the update with 409 Conflict
    // if the resource has been modified since the client last read it.
    let mut replaced_meta = current.object_meta.clone().unwrap_or_default();
    if let Some(client_rv) = replacement
        .object_meta
        .as_ref()
        .and_then(|m| m.resource_version.clone())
    {
        replaced_meta.resource_version = Some(client_rv);
    }

    let replaced = Lease {
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
