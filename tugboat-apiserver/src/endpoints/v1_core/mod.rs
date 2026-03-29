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

mod clusternetworkclass;
mod configmap;
mod namespace;
mod networkclass;
mod node;
mod persistent_volume;
mod persistent_volume_claim;
mod secret;
mod ship;
mod shipclass;
mod storage_class;

#[derive(OpenApi)]
#[openapi(
    paths(
        clusternetworkclass::handle_clusternetworkclass_create,
        clusternetworkclass::handle_clusternetworkclass_delete,
        clusternetworkclass::handle_clusternetworkclass_list,
        clusternetworkclass::handle_clusternetworkclass_read,
        clusternetworkclass::handle_clusternetworkclass_status_patch,
        clusternetworkclass::handle_clusternetworkclass_status_replace,
        configmap::handle_configmap_create,
        configmap::handle_configmap_delete,
        configmap::handle_configmap_list,
        configmap::handle_configmap_list_all,
        configmap::handle_configmap_read,
        configmap::handle_configmap_replace,
        namespace::handle_namespace_create,
        namespace::handle_namespace_delete,
        namespace::handle_namespace_list,
        namespace::handle_namespace_read,
        node::handle_node_create,
        node::handle_node_delete,
        node::handle_node_list,
        node::handle_node_read,
        node::handle_node_replace,
        node::handle_node_status_patch,
        node::handle_node_status_replace,
        persistent_volume::handle_persistent_volume_create,
        persistent_volume::handle_persistent_volume_delete,
        persistent_volume::handle_persistent_volume_list,
        persistent_volume::handle_persistent_volume_read,
        persistent_volume::handle_persistent_volume_replace,
        persistent_volume::handle_persistent_volume_status_patch,
        persistent_volume::handle_persistent_volume_status_replace,
        persistent_volume_claim::handle_persistent_volume_claim_create,
        persistent_volume_claim::handle_persistent_volume_claim_delete,
        persistent_volume_claim::handle_persistent_volume_claim_list,
        persistent_volume_claim::handle_persistent_volume_claim_list_all,
        persistent_volume_claim::handle_persistent_volume_claim_read,
        persistent_volume_claim::handle_persistent_volume_claim_replace,
        persistent_volume_claim::handle_persistent_volume_claim_status_patch,
        persistent_volume_claim::handle_persistent_volume_claim_status_replace,
        networkclass::handle_networkclass_create,
        networkclass::handle_networkclass_delete,
        networkclass::handle_networkclass_list,
        networkclass::handle_networkclass_list_all,
        networkclass::handle_networkclass_read,
        networkclass::handle_networkclass_status_patch,
        networkclass::handle_networkclass_status_replace,
        secret::handle_secret_create,
        secret::handle_secret_delete,
        secret::handle_secret_list,
        secret::handle_secret_list_all,
        secret::handle_secret_read,
        secret::handle_secret_replace,
        storage_class::handle_storage_class_create,
        storage_class::handle_storage_class_delete,
        storage_class::handle_storage_class_list,
        storage_class::handle_storage_class_read,
        ship::handle_ship_create,
        ship::handle_ship_delete,
        ship::handle_ship_list,
        ship::handle_ship_list_all,
        ship::handle_ship_read,
        ship::handle_ship_replace,
        ship::handle_ship_status_patch,
        ship::handle_ship_status_replace,
        shipclass::handle_shipclass_create,
        shipclass::handle_shipclass_delete,
        shipclass::handle_shipclass_list,
        shipclass::handle_shipclass_read,
    ),
    components(schemas(
        tugboat_resources::manifests::core::v1::Namespace,
        tugboat_resources::manifests::core::v1::PersistentVolume,
        tugboat_resources::manifests::core::v1::PersistentVolumeClaim,
        tugboat_resources::manifests::core::v1::ConfigMap,
        tugboat_resources::manifests::core::v1::Node,
        tugboat_resources::manifests::core::v1::Secret,
        tugboat_resources::manifests::core::v1::StorageClass,
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
        .service(clusternetworkclass::handle_clusternetworkclass_create)
        .service(clusternetworkclass::handle_clusternetworkclass_delete)
        .service(clusternetworkclass::handle_clusternetworkclass_list)
        .service(clusternetworkclass::handle_clusternetworkclass_read)
        .service(clusternetworkclass::handle_clusternetworkclass_status_patch)
        .service(clusternetworkclass::handle_clusternetworkclass_status_replace);
}

pub(super) fn register_configmap(service: &mut ServiceConfig) {
    service
        .service(configmap::handle_configmap_create)
        .service(configmap::handle_configmap_delete)
        .service(configmap::handle_configmap_list)
        .service(configmap::handle_configmap_list_all)
        .service(configmap::handle_configmap_read)
        .service(configmap::handle_configmap_replace);
}

pub(super) fn register_namespace(service: &mut ServiceConfig) {
    service
        .service(namespace::handle_namespace_create)
        .service(namespace::handle_namespace_delete)
        .service(namespace::handle_namespace_list)
        .service(namespace::handle_namespace_read);
}

pub(super) fn register_node(service: &mut ServiceConfig) {
    service
        .service(node::handle_node_create)
        .service(node::handle_node_delete)
        .service(node::handle_node_list)
        .service(node::handle_node_read)
        .service(node::handle_node_replace)
        .service(node::handle_node_status_patch)
        .service(node::handle_node_status_replace);
}

pub(super) fn register_persistent_volume(service: &mut ServiceConfig) {
    service
        .service(persistent_volume::handle_persistent_volume_create)
        .service(persistent_volume::handle_persistent_volume_delete)
        .service(persistent_volume::handle_persistent_volume_list)
        .service(persistent_volume::handle_persistent_volume_read)
        .service(persistent_volume::handle_persistent_volume_replace)
        .service(persistent_volume::handle_persistent_volume_status_patch)
        .service(persistent_volume::handle_persistent_volume_status_replace);
}

pub(super) fn register_persistent_volume_claim(service: &mut ServiceConfig) {
    service
        .service(persistent_volume_claim::handle_persistent_volume_claim_create)
        .service(persistent_volume_claim::handle_persistent_volume_claim_delete)
        .service(persistent_volume_claim::handle_persistent_volume_claim_list)
        .service(persistent_volume_claim::handle_persistent_volume_claim_list_all)
        .service(persistent_volume_claim::handle_persistent_volume_claim_read)
        .service(persistent_volume_claim::handle_persistent_volume_claim_replace)
        .service(persistent_volume_claim::handle_persistent_volume_claim_status_patch)
        .service(persistent_volume_claim::handle_persistent_volume_claim_status_replace);
}

pub(super) fn register_networkclass(service: &mut ServiceConfig) {
    service
        .service(networkclass::handle_networkclass_create)
        .service(networkclass::handle_networkclass_delete)
        .service(networkclass::handle_networkclass_list)
        .service(networkclass::handle_networkclass_list_all)
        .service(networkclass::handle_networkclass_read)
        .service(networkclass::handle_networkclass_status_patch)
        .service(networkclass::handle_networkclass_status_replace);
}

pub(super) fn register_secret(service: &mut ServiceConfig) {
    service
        .service(secret::handle_secret_create)
        .service(secret::handle_secret_delete)
        .service(secret::handle_secret_list)
        .service(secret::handle_secret_list_all)
        .service(secret::handle_secret_read)
        .service(secret::handle_secret_replace);
}

pub(super) fn register_storage_class(service: &mut ServiceConfig) {
    service
        .service(storage_class::handle_storage_class_create)
        .service(storage_class::handle_storage_class_delete)
        .service(storage_class::handle_storage_class_list)
        .service(storage_class::handle_storage_class_read);
}

pub(super) fn register_ship(service: &mut ServiceConfig) {
    service
        .service(ship::handle_ship_create)
        .service(ship::handle_ship_delete)
        .service(ship::handle_ship_list)
        .service(ship::handle_ship_list_all)
        .service(ship::handle_ship_read)
        .service(ship::handle_ship_replace)
        .service(ship::handle_ship_status_patch)
        .service(ship::handle_ship_status_replace);
}

pub(super) fn register_shipclass(service: &mut ServiceConfig) {
    service
        .service(shipclass::handle_shipclass_create)
        .service(shipclass::handle_shipclass_delete)
        .service(shipclass::handle_shipclass_list)
        .service(shipclass::handle_shipclass_read);
}

#[allow(dead_code)]
pub(super) fn register_v1_core(service: &mut ServiceConfig) {
    service
        .configure(register_clusternetworkclass)
        .configure(register_configmap)
        .configure(register_namespace)
        .configure(register_node)
        .configure(register_persistent_volume)
        .configure(register_persistent_volume_claim)
        .configure(register_networkclass)
        .configure(register_secret)
        .configure(register_storage_class)
        .configure(register_ship)
        .configure(register_shipclass);
}
