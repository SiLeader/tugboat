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

use actix_web::{HttpResponse, Responder, get};
use utoipa::OpenApi;
use utoipa_actix_web::service_config::ServiceConfig;

mod lease;

#[derive(OpenApi)]
#[openapi(
    paths(
        lease::handle_lease_create,
        lease::handle_lease_list,
        lease::handle_lease_list_all,
        lease::handle_lease_patch,
        lease::handle_lease_read,
        lease::handle_lease_replace,
    ),
    components(schemas(
        tugboat_resources::manifests::coordination::v1::Lease,
        tugboat_resources::manifests::meta::v1::ObjectMeta,
        tugboat_resources::manifests::meta::v1::TypeMeta,
        tugboat_resources::manifests::meta::v1::Time,
    ))
)]
struct CoordinationV1ApiDoc;

#[get("/openapi/v3/apis/coordination/v1")]
pub(crate) async fn openapi_coordination_v1() -> impl Responder {
    HttpResponse::Ok().json(CoordinationV1ApiDoc::openapi())
}

pub(super) fn register_lease(service: &mut ServiceConfig) {
    service
        .service(lease::handle_lease_create)
        .service(lease::handle_lease_list)
        .service(lease::handle_lease_list_all)
        .service(lease::handle_lease_patch)
        .service(lease::handle_lease_read)
        .service(lease::handle_lease_replace);
}

#[allow(dead_code)]
pub(super) fn register_v1_coordination(service: &mut ServiceConfig) {
    service.configure(register_lease);
}
