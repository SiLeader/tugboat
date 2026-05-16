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

use crate::crd_schema::validate_crd_schemas;
use crate::data::{ModifyResponse, ReadResponse, StatusResponse};
use crate::endpoints::resource_handlers::ReplaceOptions;
use crate::endpoints::{ClusterNamePathParams, ListQuery, resource_handlers};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, patch, post, put};
use tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinition;
use tugboat_resources::validators::Validatable;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = CustomResourceDefinition),
            (status = 400, description = "Invalid resource", body = StatusResponse),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        request_body = CustomResourceDefinition
    )]
#[post("/apis/apiextensions/v1/customresourcedefinitions")]
pub(super) async fn handle_custom_resource_definition_create(
    json: Json<CustomResourceDefinition>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    let crd = json.into_inner();
    validate_custom_resource_definition_schema(&crd)?;
    resource_handlers::create_cluster(crd, operator).await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = CustomResourceDefinition),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource"))
    )]
#[delete("/apis/apiextensions/v1/customresourcedefinitions/{name}")]
pub(super) async fn handle_custom_resource_definition_delete(
    path: Path<ClusterNamePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    resource_handlers::delete_resource::<CustomResourceDefinition>(
        &operator,
        None,
        path.into_inner().name,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [CustomResourceDefinition]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/apis/apiextensions/v1/customresourcedefinitions")]
pub(super) async fn handle_custom_resource_definition_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<CustomResourceDefinition>(
        &operator,
        query.into_inner(),
        None,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = CustomResourceDefinition),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource"))
    )]
#[get("/apis/apiextensions/v1/customresourcedefinitions/{name}")]
pub(super) async fn handle_custom_resource_definition_read(
    path: Path<ClusterNamePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    resource_handlers::read_resource::<CustomResourceDefinition>(
        &operator,
        None,
        path.into_inner().name,
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = CustomResourceDefinition),
            (status = 400, description = "Invalid resource", body = StatusResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = CustomResourceDefinition
    )]
#[put("/apis/apiextensions/v1/customresourcedefinitions/{name}")]
pub(super) async fn handle_custom_resource_definition_replace(
    path: Path<ClusterNamePathParams>,
    replacement: Json<CustomResourceDefinition>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    let replacement = replacement.into_inner();
    validate_custom_resource_definition_schema(&replacement)?;
    resource_handlers::replace_resource::<CustomResourceDefinition>(
        &operator,
        None,
        path.into_inner().name,
        replacement,
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = CustomResourceDefinition),
            (status = 400, description = "Invalid resource", body = StatusResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = Object
    )]
#[patch("/apis/apiextensions/v1/customresourcedefinitions/{name}")]
pub(super) async fn handle_custom_resource_definition_patch(
    path: Path<ClusterNamePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    resource_handlers::patch_resource::<CustomResourceDefinition>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = CustomResourceDefinition),
            (status = 400, description = "Invalid resource", body = StatusResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = Object
    )]
#[patch("/apis/apiextensions/v1/customresourcedefinitions/{name}/status")]
pub(super) async fn handle_custom_resource_definition_status_patch(
    path: Path<ClusterNamePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    resource_handlers::status_patch_resource::<CustomResourceDefinition>(
        &operator,
        None,
        path.into_inner().name,
        patch.into_inner(),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = CustomResourceDefinition),
            (status = 400, description = "Invalid resource", body = StatusResponse),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(("name" = String, Path, description = "Name of the resource")),
        request_body = CustomResourceDefinition
    )]
#[put("/apis/apiextensions/v1/customresourcedefinitions/{name}/status")]
pub(super) async fn handle_custom_resource_definition_status_replace(
    path: Path<ClusterNamePathParams>,
    replacement: Json<CustomResourceDefinition>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<CustomResourceDefinition>, Box<StatusResponse>> {
    resource_handlers::status_replace_resource::<CustomResourceDefinition>(
        &operator,
        None,
        path.into_inner().name,
        replacement.into_inner(),
    )
    .await
}

fn validate_custom_resource_definition_schema(
    crd: &CustomResourceDefinition,
) -> Result<(), Box<StatusResponse>> {
    validate_crd_schemas(crd).map_err(|err| {
        Box::new(StatusResponse::invalid(
            format!("Invalid CustomResourceDefinition schema: {err}"),
            Some(serde_json::json!({ "reason": err })),
        ))
    })?;
    if crd.validate() {
        Ok(())
    } else {
        Err(Box::new(StatusResponse::invalid(
            "Invalid CustomResourceDefinition resource",
            None,
        )))
    }
}
