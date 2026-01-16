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
use crate::{check_namespace_absent, create_object, extract_object_meta};
use actix_web::post;
use actix_web::web::{Data, Json};
use tugboat_resources::Resource;
use tugboat_resources::manifests::core::v1::Namespace;

#[utoipa::path()]
#[post("/v1/namespaces")]
pub(super) async fn handle_namespace_create(
    json: Json<Namespace>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Namespace>, StatusResponse> {
    let namespace = json.into_inner();

    let object_meta = extract_object_meta!(namespace);
    check_namespace_absent!(object_meta);

    create_object!(operator, object_meta, namespace, Namespace::type_meta())
}
