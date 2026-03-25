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
use crate::endpoints::{ListQuery, resource_handlers};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, post};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::ShipClass;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = ShipClass),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        request_body = ShipClass
    )]
#[post("/api/v1/shipclasses")]
pub(super) async fn handle_shipclass_create(
    json: Json<ShipClass>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ShipClass>, Box<StatusResponse>> {
    resource_handlers::create_cluster(json.into_inner(), operator).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipClassDeletePathParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = ShipClass),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/shipclasses/{name}")]
pub(super) async fn handle_shipclass_delete(
    path: Path<ShipClassDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ShipClass>, Box<StatusResponse>> {
    resource_handlers::delete_resource::<ShipClass>(&operator, None, path.into_inner().name).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [ShipClass]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/shipclasses")]
pub(super) async fn handle_shipclass_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<ShipClass>(&operator, query.into_inner(), None).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ReadParams {
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = ShipClass),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/shipclasses/{name}")]
pub(super) async fn handle_shipclass_read(
    path: Path<ReadParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ShipClass>, Box<StatusResponse>> {
    resource_handlers::read_resource::<ShipClass>(&operator, None, path.into_inner().name).await
}
