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

use crate::data::{ModifyResponse, ReadResponse, StatusResponse};
use crate::endpoints::resource_handlers;
use crate::endpoints::resource_handlers::ReplaceOptions;
use crate::endpoints::{ListQuery, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpResponse, delete, get, post, put};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::ConfigMap;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = ConfigMap),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
        ),
        request_body = ConfigMap
    )]
#[post("/api/v1/namespaces/{namespace}/configmaps")]
pub(super) async fn handle_configmap_create(
    path: Path<NamespacedPathParams>,
    json: Json<ConfigMap>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ConfigMap>, Box<StatusResponse>> {
    resource_handlers::create_namespaced(json.into_inner(), path.into_inner().namespace, operator)
        .await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ConfigMapDeletePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = ConfigMap),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/namespaces/{namespace}/configmaps/{name}")]
pub(super) async fn handle_configmap_delete(
    path: Path<ConfigMapDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ConfigMap>, Box<StatusResponse>> {
    let params = path.into_inner();
    resource_handlers::delete_resource::<ConfigMap>(&operator, Some(params.namespace), params.name)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [ConfigMap]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/namespaces/{namespace}/configmaps")]
pub(super) async fn handle_configmap_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<ConfigMap>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [ConfigMap]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/configmaps")]
pub(super) async fn handle_configmap_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<ConfigMap>(&operator, query.into_inner(), None).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ConfigMapReadPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = ConfigMap),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/namespaces/{namespace}/configmaps/{name}")]
pub(super) async fn handle_configmap_read(
    path: Path<ConfigMapReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ConfigMap>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::read_resource::<ConfigMap>(&operator, Some(path.namespace), path.name).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ConfigMapReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = ConfigMap),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = ConfigMap
    )]
#[put("/api/v1/namespaces/{namespace}/configmaps/{name}")]
pub(super) async fn handle_configmap_replace(
    path: Path<ConfigMapReplacePathParams>,
    replacement: Json<ConfigMap>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<ConfigMap>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::replace_resource::<ConfigMap>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement.into_inner(),
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
        },
    )
    .await
}
