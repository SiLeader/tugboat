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

mod dispatch;
mod validation;
mod watch;

use crate::data::StatusResponse;
use crate::endpoints::ListQuery;
use crate::operator::ApiOperator;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put};
use serde::Deserialize;
use utoipa_actix_web::service_config::ServiceConfig;

#[derive(Deserialize)]
struct NamespacedCollectionPath {
    group: String,
    version: String,
    namespace: String,
    plural: String,
}

#[derive(Deserialize)]
struct NamespacedNamePath {
    group: String,
    version: String,
    namespace: String,
    plural: String,
    name: String,
}

#[derive(Deserialize)]
struct ClusterCollectionPath {
    group: String,
    version: String,
    plural: String,
}

#[derive(Deserialize)]
struct ClusterNamePath {
    group: String,
    version: String,
    plural: String,
    name: String,
}

pub(crate) fn register_custom_resource_routes(service: &mut ServiceConfig) {
    service.map(|config| {
        config
            .service(handle_namespaced_create)
            .service(handle_namespaced_list)
            .service(handle_namespaced_read)
            .service(handle_namespaced_replace)
            .service(handle_namespaced_patch)
            .service(handle_namespaced_delete)
            .service(handle_namespaced_status_replace)
            .service(handle_namespaced_status_patch)
            .service(handle_cluster_create)
            .service(handle_cluster_list)
            .service(handle_cluster_read)
            .service(handle_cluster_replace)
            .service(handle_cluster_patch)
            .service(handle_cluster_delete)
            .service(handle_cluster_status_replace)
            .service(handle_cluster_status_patch)
    });
}

#[post("/apis/{group}/{version}/namespaces/{namespace}/{plural}")]
async fn handle_namespaced_create(
    path: Path<NamespacedCollectionPath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::create(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        body.into_inner(),
    )
    .await
}

#[get("/apis/{group}/{version}/namespaces/{namespace}/{plural}")]
async fn handle_namespaced_list(
    path: Path<NamespacedCollectionPath>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::list(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        query.into_inner(),
    )
    .await
}

#[get("/apis/{group}/{version}/namespaces/{namespace}/{plural}/{name}")]
async fn handle_namespaced_read(
    path: Path<NamespacedNamePath>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::read(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        path.name,
    )
    .await
}

#[put("/apis/{group}/{version}/namespaces/{namespace}/{plural}/{name}")]
async fn handle_namespaced_replace(
    path: Path<NamespacedNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::replace(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[patch("/apis/{group}/{version}/namespaces/{namespace}/{plural}/{name}")]
async fn handle_namespaced_patch(
    req: HttpRequest,
    path: Path<NamespacedNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    dispatch::ensure_merge_patch_content_type(&req)?;
    let path = path.into_inner();
    dispatch::patch(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[delete("/apis/{group}/{version}/namespaces/{namespace}/{plural}/{name}")]
async fn handle_namespaced_delete(
    path: Path<NamespacedNamePath>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::delete(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        path.name,
    )
    .await
}

#[put("/apis/{group}/{version}/namespaces/{namespace}/{plural}/{name}/status")]
async fn handle_namespaced_status_replace(
    path: Path<NamespacedNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::replace_status(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[patch("/apis/{group}/{version}/namespaces/{namespace}/{plural}/{name}/status")]
async fn handle_namespaced_status_patch(
    req: HttpRequest,
    path: Path<NamespacedNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    dispatch::ensure_merge_patch_content_type(&req)?;
    let path = path.into_inner();
    dispatch::patch_status(
        &operator,
        path.group,
        path.version,
        Some(path.namespace),
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[post("/apis/{group}/{version}/{plural}")]
async fn handle_cluster_create(
    path: Path<ClusterCollectionPath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::create(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        body.into_inner(),
    )
    .await
}

#[get("/apis/{group}/{version}/{plural}")]
async fn handle_cluster_list(
    path: Path<ClusterCollectionPath>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::list(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        query.into_inner(),
    )
    .await
}

#[get("/apis/{group}/{version}/{plural}/{name}")]
async fn handle_cluster_read(
    path: Path<ClusterNamePath>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::read(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        path.name,
    )
    .await
}

#[put("/apis/{group}/{version}/{plural}/{name}")]
async fn handle_cluster_replace(
    path: Path<ClusterNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::replace(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[patch("/apis/{group}/{version}/{plural}/{name}")]
async fn handle_cluster_patch(
    req: HttpRequest,
    path: Path<ClusterNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    dispatch::ensure_merge_patch_content_type(&req)?;
    let path = path.into_inner();
    dispatch::patch(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[delete("/apis/{group}/{version}/{plural}/{name}")]
async fn handle_cluster_delete(
    path: Path<ClusterNamePath>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::delete(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        path.name,
    )
    .await
}

#[put("/apis/{group}/{version}/{plural}/{name}/status")]
async fn handle_cluster_status_replace(
    path: Path<ClusterNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    dispatch::replace_status(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}

#[patch("/apis/{group}/{version}/{plural}/{name}/status")]
async fn handle_cluster_status_patch(
    req: HttpRequest,
    path: Path<ClusterNamePath>,
    body: Json<serde_json::Value>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    dispatch::ensure_merge_patch_content_type(&req)?;
    let path = path.into_inner();
    dispatch::patch_status(
        &operator,
        path.group,
        path.version,
        None,
        path.plural,
        path.name,
        body.into_inner(),
    )
    .await
}
