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
use crate::operator::ApiOperator;
use actix_web::web::Data;
use actix_web::{HttpResponse, get};

#[get("/openid/v1/jwks")]
pub(crate) async fn jwks(operator: Data<ApiOperator>) -> Result<HttpResponse, Box<StatusResponse>> {
    let Some(issuer) = &operator.service_account_tokens else {
        return Err(Box::new(StatusResponse::not_found(
            "ServiceAccount JWT signing is not configured",
            None,
        )));
    };
    Ok(HttpResponse::Ok().json(issuer.jwks()))
}

#[get("/.well-known/openid-configuration")]
pub(crate) async fn openid_configuration(
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let Some(issuer) = &operator.service_account_tokens else {
        return Err(Box::new(StatusResponse::not_found(
            "ServiceAccount JWT signing is not configured",
            None,
        )));
    };
    Ok(HttpResponse::Ok().json(issuer.openid_configuration()))
}
