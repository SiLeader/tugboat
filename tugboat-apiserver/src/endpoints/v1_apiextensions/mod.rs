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

mod custom_resource_definition;

#[derive(OpenApi)]
#[openapi(
    paths(
        custom_resource_definition::handle_custom_resource_definition_create,
        custom_resource_definition::handle_custom_resource_definition_delete,
        custom_resource_definition::handle_custom_resource_definition_list,
        custom_resource_definition::handle_custom_resource_definition_patch,
        custom_resource_definition::handle_custom_resource_definition_read,
        custom_resource_definition::handle_custom_resource_definition_replace,
        custom_resource_definition::handle_custom_resource_definition_status_patch,
        custom_resource_definition::handle_custom_resource_definition_status_replace,
    ),
    components(schemas(
        tugboat_resources::manifests::apiextensions::v1::Condition,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinition,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinitionNames,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinitionSpec,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinitionStatus,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinitionVersion,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceSubresourceStatus,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceSubresources,
        tugboat_resources::manifests::apiextensions::v1::CustomResourceValidation,
        tugboat_resources::manifests::meta::v1::ObjectMeta,
        tugboat_resources::manifests::meta::v1::TypeMeta,
        tugboat_resources::manifests::meta::v1::Time,
    ))
)]
struct ApiextensionsV1ApiDoc;

#[get("/openapi/v3/apis/apiextensions/v1")]
pub(crate) async fn openapi_apiextensions_v1() -> impl Responder {
    HttpResponse::Ok().json(ApiextensionsV1ApiDoc::openapi())
}

pub(super) fn register_custom_resource_definition(service: &mut ServiceConfig) {
    service
        .service(custom_resource_definition::handle_custom_resource_definition_create)
        .service(custom_resource_definition::handle_custom_resource_definition_delete)
        .service(custom_resource_definition::handle_custom_resource_definition_list)
        .service(custom_resource_definition::handle_custom_resource_definition_patch)
        .service(custom_resource_definition::handle_custom_resource_definition_read)
        .service(custom_resource_definition::handle_custom_resource_definition_replace)
        .service(custom_resource_definition::handle_custom_resource_definition_status_patch)
        .service(custom_resource_definition::handle_custom_resource_definition_status_replace);
}
