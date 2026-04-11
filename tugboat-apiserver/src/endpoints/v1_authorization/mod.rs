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

mod cluster_role;
mod cluster_role_binding;
mod role;
mod role_binding;

#[derive(OpenApi)]
#[openapi(
    paths(
        cluster_role::handle_cluster_role_create,
        cluster_role::handle_cluster_role_delete,
        cluster_role::handle_cluster_role_list,
        cluster_role::handle_cluster_role_patch,
        cluster_role::handle_cluster_role_read,
        cluster_role::handle_cluster_role_replace,
        cluster_role_binding::handle_cluster_role_binding_create,
        cluster_role_binding::handle_cluster_role_binding_delete,
        cluster_role_binding::handle_cluster_role_binding_list,
        cluster_role_binding::handle_cluster_role_binding_patch,
        cluster_role_binding::handle_cluster_role_binding_read,
        cluster_role_binding::handle_cluster_role_binding_replace,
        role::handle_role_create,
        role::handle_role_delete,
        role::handle_role_list,
        role::handle_role_list_all,
        role::handle_role_patch,
        role::handle_role_read,
        role::handle_role_replace,
        role_binding::handle_role_binding_create,
        role_binding::handle_role_binding_delete,
        role_binding::handle_role_binding_list,
        role_binding::handle_role_binding_list_all,
        role_binding::handle_role_binding_patch,
        role_binding::handle_role_binding_read,
        role_binding::handle_role_binding_replace,
    ),
    components(schemas(
        tugboat_resources::manifests::authorization::v1::ClusterRole,
        tugboat_resources::manifests::authorization::v1::ClusterRoleBinding,
        tugboat_resources::manifests::authorization::v1::PolicyRule,
        tugboat_resources::manifests::authorization::v1::Role,
        tugboat_resources::manifests::authorization::v1::RoleBinding,
        tugboat_resources::manifests::authorization::v1::RoleRef,
        tugboat_resources::manifests::authorization::v1::Subject,
        tugboat_resources::manifests::meta::v1::ObjectMeta,
        tugboat_resources::manifests::meta::v1::TypeMeta,
    ))
)]
struct AuthorizationV1ApiDoc;

#[get("/openapi/v3/apis/authorization/v1")]
pub(crate) async fn openapi_authorization_v1() -> impl Responder {
    HttpResponse::Ok().json(AuthorizationV1ApiDoc::openapi())
}

pub(super) fn register_cluster_role(service: &mut ServiceConfig) {
    service
        .service(cluster_role::handle_cluster_role_create)
        .service(cluster_role::handle_cluster_role_delete)
        .service(cluster_role::handle_cluster_role_list)
        .service(cluster_role::handle_cluster_role_patch)
        .service(cluster_role::handle_cluster_role_read)
        .service(cluster_role::handle_cluster_role_replace);
}

pub(super) fn register_cluster_role_binding(service: &mut ServiceConfig) {
    service
        .service(cluster_role_binding::handle_cluster_role_binding_create)
        .service(cluster_role_binding::handle_cluster_role_binding_delete)
        .service(cluster_role_binding::handle_cluster_role_binding_list)
        .service(cluster_role_binding::handle_cluster_role_binding_patch)
        .service(cluster_role_binding::handle_cluster_role_binding_read)
        .service(cluster_role_binding::handle_cluster_role_binding_replace);
}

pub(super) fn register_role(service: &mut ServiceConfig) {
    service
        .service(role::handle_role_create)
        .service(role::handle_role_delete)
        .service(role::handle_role_list)
        .service(role::handle_role_list_all)
        .service(role::handle_role_patch)
        .service(role::handle_role_read)
        .service(role::handle_role_replace);
}

pub(super) fn register_role_binding(service: &mut ServiceConfig) {
    service
        .service(role_binding::handle_role_binding_create)
        .service(role_binding::handle_role_binding_delete)
        .service(role_binding::handle_role_binding_list)
        .service(role_binding::handle_role_binding_list_all)
        .service(role_binding::handle_role_binding_patch)
        .service(role_binding::handle_role_binding_read)
        .service(role_binding::handle_role_binding_replace);
}
