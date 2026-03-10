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
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use tugboat_resources::manifests::core::v1::Secret;

#[utoipa::path(
    responses(
        (status = 200, description = "Resource created", body = Secret),
        (status = 409, description = "Resource already exists", body = StatusResponse),
        (status = 500, description = "Internal server error", body = StatusResponse),
    ),
    params(
        ("namespace" = String, Path, description = "Namespace of the resource"),
    ),
    request_body = Secret
)]
#[post("/api/v1/namespaces/{namespace}/secrets")]
pub(super) async fn handle_secret_create(
    path: Path<NamespacedPathParams>,
    json: Json<Secret>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Secret>, StatusResponse> {
    let json = json.into_inner();
    let mut data = json.data;
    data.extend(
        json.string_data
            .into_iter()
            .map(|(k, v)| (k, base64_encode(&v))),
    );
    let json = Secret {
        data,
        string_data: Default::default(),
        ..json
    };
    resource_handlers::create_namespaced(json, path.into_inner().namespace, operator).await
}

fn base64_encode(s: &str) -> String {
    let mut str = String::with_capacity(s.len());
    BASE64_STANDARD.encode_string(s, &mut str);
    str
}
