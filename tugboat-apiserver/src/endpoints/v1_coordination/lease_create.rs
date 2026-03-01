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
use crate::endpoints::NamespacedPathParams;
use crate::endpoints::resource_handlers;
use crate::operator::ApiOperator;
use actix_web::post;
use actix_web::web::{Data, Json, Path};
use tugboat_resources::manifests::coordination::v1::Lease;

#[utoipa::path()]
#[post("/apis/coordination/v1/namespaces/{namespace}/leases")]
pub(super) async fn handle_lease_create(
    path: Path<NamespacedPathParams>,
    json: Json<Lease>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Lease>, StatusResponse> {
    resource_handlers::create_namespaced(json.into_inner(), path.into_inner().namespace, operator)
        .await
}
