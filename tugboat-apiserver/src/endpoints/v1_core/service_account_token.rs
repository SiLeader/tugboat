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

use crate::auth::service_account_jwt::{ServiceAccountTokenRequest, ServiceAccountTokenResponse};
use crate::data::StatusResponse;
use crate::endpoints::NamespacedNamePathParams;
use crate::operator::ApiOperator;
use actix_web::post;
use actix_web::web::{Data, Json, Path};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::ServiceAccount;

#[utoipa::path(
        responses(
            (status = 200, description = "ServiceAccount token issued", body = ServiceAccountTokenResponse),
            (status = 400, description = "Invalid token request", body = StatusResponse),
            (status = 404, description = "ServiceAccount not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the ServiceAccount"),
            ("name" = String, Path, description = "Name of the ServiceAccount"),
        ),
        request_body = ServiceAccountTokenRequest
    )]
#[post("/api/v1/namespaces/{namespace}/serviceaccounts/{name}/token")]
pub(super) async fn handle_service_account_token_create(
    path: Path<NamespacedNamePathParams>,
    json: Json<ServiceAccountTokenRequest>,
    operator: Data<ApiOperator>,
) -> Result<Json<ServiceAccountTokenResponse>, Box<StatusResponse>> {
    let Some(issuer) = &operator.service_account_tokens else {
        return Err(Box::new(StatusResponse::bad_request(
            "ServiceAccount JWT signing is not configured",
            None,
        )));
    };
    let path = path.into_inner();
    let service_account = operator
        .store
        .get::<ServiceAccount>(Some(path.namespace.clone()), &path.name)
        .await?
        .map(|data| data.apply_revision())
        .ok_or_else(|| {
            Box::new(StatusResponse::not_found(
                "ServiceAccount not found",
                Some(serde_json::json!({
                    "namespace": path.namespace,
                    "name": path.name,
                })),
            ))
        })?;
    if service_account.deletion_timestamp().is_some() {
        return Err(Box::new(StatusResponse::bad_request(
            "Cannot issue a token for a deleting ServiceAccount",
            None,
        )));
    }
    let response = issuer
        .issue_token(&service_account, json.into_inner())
        .map_err(|err| Box::new(StatusResponse::bad_request(err, None)))?;
    Ok(Json(response))
}
