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
use crate::operator::ApiOperator;
use crate::{create_object, extract_object_meta};
use actix_web::post;
use actix_web::web::{Data, Json, Path};
use tugboat_resources::Resource;
use tugboat_resources::manifests::core::v1::Ship;

#[utoipa::path()]
#[post("/v1/namespaces/{namespace}/ships")]
pub(super) async fn handle_ship_create(
    path: Path<NamespacedPathParams>,
    json: Json<Ship>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Ship>, StatusResponse> {
    let ship = json.into_inner();

    let object_meta = extract_object_meta!(ship);
    let object_meta = operator.apply_namespace(object_meta, path.into_inner().namespace);

    create_object!(operator, object_meta, ship, Ship::type_meta())
}
