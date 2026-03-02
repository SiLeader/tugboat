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

mod clusternetworkclass_create;
mod clusternetworkclass_list;
mod clusternetworkclass_read;
mod namespace_create;
mod namespace_list;
mod namespace_read;
mod networkclass_create;
mod networkclass_list;
mod networkclass_read;
mod node_create;
mod node_delete;
mod node_list;
mod node_read;
mod node_replace;
mod ship_create;
mod ship_list;
mod ship_read;
mod ship_replace;
mod ship_status_patch;
mod ship_status_replace;
mod shipclass_create;
mod shipclass_list;
mod shipclass_read;

#[derive(OpenApi)]
#[openapi(
    paths(
        clusternetworkclass_create::handle_clusternetworkclass_create,
        clusternetworkclass_list::handle_clusternetworkclass_list,
        clusternetworkclass_read::handle_clusternetworkclass_read,
        namespace_create::handle_namespace_create,
        namespace_list::handle_namespace_list,
        namespace_read::handle_namespace_read,
        node_create::handle_node_create,
        node_delete::handle_node_delete,
        node_list::handle_node_list,
        node_read::handle_node_read,
        node_replace::handle_node_replace,
        networkclass_create::handle_networkclass_create,
        networkclass_list::handle_networkclass_list,
        networkclass_list::handle_networkclass_list_all,
        networkclass_read::handle_networkclass_read,
        ship_create::handle_ship_create,
        ship_list::handle_ship_list,
        ship_list::handle_ship_list_all,
        ship_read::handle_ship_read,
        ship_replace::handle_ship_replace,
        ship_status_patch::handle_ship_status_patch,
        ship_status_replace::handle_ship_status_replace,
        shipclass_create::handle_shipclass_create,
        shipclass_list::handle_shipclass_list,
        shipclass_read::handle_shipclass_read,
    ),
    components(schemas(
        tugboat_resources::manifests::core::v1::Namespace,
        tugboat_resources::manifests::core::v1::Node,
        tugboat_resources::manifests::core::v1::Ship,
        tugboat_resources::manifests::core::v1::ShipClass,
        tugboat_resources::manifests::core::v1::NetworkClass,
        tugboat_resources::manifests::core::v1::ClusterNetworkClass,
        tugboat_resources::manifests::meta::v1::ObjectMeta,
        tugboat_resources::manifests::meta::v1::TypeMeta,
        tugboat_resources::manifests::meta::v1::Time,
    ))
)]
struct CoreV1ApiDoc;

#[get("/openapi/v3/api/v1")]
pub(crate) async fn openapi_core_v1() -> impl Responder {
    HttpResponse::Ok().json(CoreV1ApiDoc::openapi())
}

pub(super) fn register_clusternetworkclass(service: &mut ServiceConfig) {
    service
        .service(clusternetworkclass_create::handle_clusternetworkclass_create)
        .service(clusternetworkclass_list::handle_clusternetworkclass_list)
        .service(clusternetworkclass_read::handle_clusternetworkclass_read);
}

pub(super) fn register_namespace(service: &mut ServiceConfig) {
    service
        .service(namespace_create::handle_namespace_create)
        .service(namespace_list::handle_namespace_list)
        .service(namespace_read::handle_namespace_read);
}

pub(super) fn register_node(service: &mut ServiceConfig) {
    service
        .service(node_create::handle_node_create)
        .service(node_delete::handle_node_delete)
        .service(node_list::handle_node_list)
        .service(node_read::handle_node_read)
        .service(node_replace::handle_node_replace);
}

pub(super) fn register_networkclass(service: &mut ServiceConfig) {
    service
        .service(networkclass_create::handle_networkclass_create)
        .service(networkclass_list::handle_networkclass_list)
        .service(networkclass_list::handle_networkclass_list_all)
        .service(networkclass_read::handle_networkclass_read);
}

pub(super) fn register_ship(service: &mut ServiceConfig) {
    service
        .service(ship_create::handle_ship_create)
        .service(ship_list::handle_ship_list)
        .service(ship_list::handle_ship_list_all)
        .service(ship_read::handle_ship_read)
        .service(ship_replace::handle_ship_replace)
        .service(ship_status_patch::handle_ship_status_patch)
        .service(ship_status_replace::handle_ship_status_replace);
}

pub(super) fn register_shipclass(service: &mut ServiceConfig) {
    service
        .service(shipclass_create::handle_shipclass_create)
        .service(shipclass_list::handle_shipclass_list)
        .service(shipclass_read::handle_shipclass_read);
}

#[allow(dead_code)]
pub(super) fn register_v1_core(service: &mut ServiceConfig) {
    service
        .configure(register_clusternetworkclass)
        .configure(register_namespace)
        .configure(register_node)
        .configure(register_networkclass)
        .configure(register_ship)
        .configure(register_shipclass);
}
