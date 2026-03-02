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

use crate::data::StatusResponse;
use crate::endpoints::resource_handlers;
use crate::endpoints::{ListQuery, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Path, Query};
use actix_web::{HttpResponse, get};
use tugboat_resources::manifests::coordination::v1::Lease;

#[utoipa::path(
    responses(
        (status = 200, description = "List of resources", body = [Lease]),
        (status = 500, description = "Internal server error", body = StatusResponse),
    ),
    params(
        ("namespace" = String, Path, description = "Namespace of the resource"),
        ("watch" = Option<String>, Query, description = "Watch for changes"),
        ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
        ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
        ("labelSelector" = Option<String>, Query, description = "Filter by label"),
    )
)]
#[get("/apis/coordination/v1/namespaces/{namespace}/leases")]
pub(super) async fn handle_lease_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    resource_handlers::list_resources::<Lease>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
    responses(
        (status = 200, description = "List of resources", body = [Lease]),
        (status = 500, description = "Internal server error", body = StatusResponse),
    ),
    params(
        ("watch" = Option<String>, Query, description = "Watch for changes"),
        ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
        ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
        ("labelSelector" = Option<String>, Query, description = "Filter by label"),
    )
)]
#[get("/apis/coordination/v1/leases")]
pub(super) async fn handle_lease_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    resource_handlers::list_resources::<Lease>(&operator, query.into_inner(), None).await
}
