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

mod deployment;
mod fleet;
mod replicaset;

#[derive(OpenApi)]
#[openapi(
    paths(
        deployment::handle_deployment_create,
        deployment::handle_deployment_delete,
        deployment::handle_deployment_list,
        deployment::handle_deployment_list_all,
        deployment::handle_deployment_patch,
        deployment::handle_deployment_read,
        deployment::handle_deployment_replace,
        fleet::handle_fleet_create,
        fleet::handle_fleet_delete,
        fleet::handle_fleet_list,
        fleet::handle_fleet_list_all,
        fleet::handle_fleet_patch,
        fleet::handle_fleet_read,
        fleet::handle_fleet_replace,
        replicaset::handle_replicaset_create,
        replicaset::handle_replicaset_delete,
        replicaset::handle_replicaset_list,
        replicaset::handle_replicaset_list_all,
        replicaset::handle_replicaset_patch,
        replicaset::handle_replicaset_read,
        replicaset::handle_replicaset_replace,
    ),
    components(schemas(
        tugboat_resources::manifests::apps::v1::Deployment,
        tugboat_resources::manifests::apps::v1::DeploymentSpec,
        tugboat_resources::manifests::apps::v1::DeploymentStatus,
        tugboat_resources::manifests::apps::v1::DeploymentStrategy,
        tugboat_resources::manifests::apps::v1::Fleet,
        tugboat_resources::manifests::apps::v1::FleetComponent,
        tugboat_resources::manifests::apps::v1::FleetSpec,
        tugboat_resources::manifests::apps::v1::FleetStatus,
        tugboat_resources::manifests::apps::v1::ReplicaSet,
        tugboat_resources::manifests::apps::v1::ReplicaSetSpec,
        tugboat_resources::manifests::apps::v1::ReplicaSetStatus,
        tugboat_resources::manifests::apps::v1::RollingUpdateStrategy,
        tugboat_resources::manifests::apps::v1::ShipTemplateSpec,
        tugboat_resources::manifests::core::v1::ShipSpec,
        tugboat_resources::manifests::meta::v1::ObjectMeta,
        tugboat_resources::manifests::meta::v1::TypeMeta,
        tugboat_resources::manifests::meta::v1::Time,
    ))
)]
struct AppsV1ApiDoc;

#[get("/openapi/v3/apis/apps/v1")]
pub(crate) async fn openapi_apps_v1() -> impl Responder {
    HttpResponse::Ok().json(AppsV1ApiDoc::openapi())
}

pub(super) fn register_deployment(service: &mut ServiceConfig) {
    service
        .service(deployment::handle_deployment_create)
        .service(deployment::handle_deployment_delete)
        .service(deployment::handle_deployment_list)
        .service(deployment::handle_deployment_list_all)
        .service(deployment::handle_deployment_patch)
        .service(deployment::handle_deployment_read)
        .service(deployment::handle_deployment_replace);
}

pub(super) fn register_fleet(service: &mut ServiceConfig) {
    service
        .service(fleet::handle_fleet_create)
        .service(fleet::handle_fleet_delete)
        .service(fleet::handle_fleet_list)
        .service(fleet::handle_fleet_list_all)
        .service(fleet::handle_fleet_patch)
        .service(fleet::handle_fleet_read)
        .service(fleet::handle_fleet_replace);
}

pub(super) fn register_replicaset(service: &mut ServiceConfig) {
    service
        .service(replicaset::handle_replicaset_create)
        .service(replicaset::handle_replicaset_delete)
        .service(replicaset::handle_replicaset_list)
        .service(replicaset::handle_replicaset_list_all)
        .service(replicaset::handle_replicaset_patch)
        .service(replicaset::handle_replicaset_read)
        .service(replicaset::handle_replicaset_replace);
}
