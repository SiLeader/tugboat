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
use actix_web::{HttpResponse, delete, get, patch, post, put};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Secret;
use utoipa::ToSchema;

#[utoipa::path(
        responses(
            (status = 200, description = "Resource created", body = Secret),
            (status = 409, description = "Resource already exists", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
        ),
        request_body = Secret
    )]
#[post("/api/v1/namespaces/{namespace}/secrets")]
pub(super) async fn handle_secret_create(
    path: Path<NamespacedPathParams>,
    json: Json<Secret>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Secret>, Box<StatusResponse>> {
    let json = json.into_inner();
    let mut data = json.data;
    data.extend(
        json.string_data
            .into_iter()
            .map(|(k, v)| (k, base64_encode(&v))),
    );
    let json = Secret {
        data,
        string_data: Default::default(),
        ..json
    };
    resource_handlers::create_namespaced(json, path.into_inner().namespace, operator).await
}

fn base64_encode(s: &str) -> String {
    let mut str = String::with_capacity(s.len());
    BASE64_STANDARD.encode_string(s, &mut str);
    str
}

fn normalize_secret_patch(
    mut patch: serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Map<String, serde_json::Value>, Box<StatusResponse>> {
    let Some(string_data) = patch.remove("stringData") else {
        return Ok(patch);
    };
    let Some(string_data) = string_data.as_object() else {
        return Err(Box::new(StatusResponse::bad_request(
            "stringData must be an object",
            None,
        )));
    };

    let mut data = patch
        .remove("data")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for (key, value) in string_data {
        let Some(value) = value.as_str() else {
            return Err(Box::new(StatusResponse::bad_request(
                "stringData values must be strings",
                None,
            )));
        };
        let _ = data.insert(key.clone(), serde_json::Value::String(base64_encode(value)));
    }
    let _ = patch.insert("data".to_string(), serde_json::Value::Object(data));
    Ok(patch)
}

#[derive(Deserialize, ToSchema)]
pub(super) struct SecretDeletePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource deleted", body = Secret),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[delete("/api/v1/namespaces/{namespace}/secrets/{name}")]
pub(super) async fn handle_secret_delete(
    path: Path<SecretDeletePathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Secret>, Box<StatusResponse>> {
    let params = path.into_inner();
    resource_handlers::delete_resource::<Secret>(&operator, Some(params.namespace), params.name)
        .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Secret]),
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
#[get("/api/v1/namespaces/{namespace}/secrets")]
pub(super) async fn handle_secret_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Secret>(
        &operator,
        query.into_inner(),
        Some(path.into_inner().namespace),
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "List of resources", body = [Secret]),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("watch" = Option<String>, Query, description = "Watch for changes"),
            ("resourceVersion" = Option<String>, Query, description = "Resource version to watch from"),
            ("fieldSelector" = Option<String>, Query, description = "Filter by field"),
            ("labelSelector" = Option<String>, Query, description = "Filter by label"),
        )
    )]
#[get("/api/v1/secrets")]
pub(super) async fn handle_secret_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    resource_handlers::list_resources::<Secret>(&operator, query.into_inner(), None).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct SecretReadPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource details", body = Secret),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        )
    )]
#[get("/api/v1/namespaces/{namespace}/secrets/{name}")]
pub(super) async fn handle_secret_read(
    path: Path<SecretReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Secret>, Box<StatusResponse>> {
    let path = path.into_inner();
    resource_handlers::read_resource::<Secret>(&operator, Some(path.namespace), path.name).await
}

#[derive(Deserialize, ToSchema)]
pub(super) struct SecretReplacePathParams {
    namespace: String,
    name: String,
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Secret),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Secret
    )]
#[put("/api/v1/namespaces/{namespace}/secrets/{name}")]
pub(super) async fn handle_secret_replace(
    path: Path<SecretReplacePathParams>,
    replacement: Json<Secret>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Secret>, Box<StatusResponse>> {
    let path = path.into_inner();
    let replacement = replacement.into_inner();
    let mut data = replacement.data;
    data.extend(
        replacement
            .string_data
            .into_iter()
            .map(|(k, v)| (k, base64_encode(&v))),
    );
    let replacement = Secret {
        data,
        string_data: Default::default(),
        ..replacement
    };
    resource_handlers::replace_resource::<Secret>(
        &operator,
        Some(path.namespace),
        path.name,
        replacement,
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .await
}

#[utoipa::path(
        responses(
            (status = 200, description = "Resource updated", body = Secret),
            (status = 404, description = "Resource not found", body = StatusResponse),
            (status = 500, description = "Internal server error", body = StatusResponse),
        ),
        params(
            ("namespace" = String, Path, description = "Namespace of the resource"),
            ("name" = String, Path, description = "Name of the resource"),
        ),
        request_body = Object
    )]
#[patch("/api/v1/namespaces/{namespace}/secrets/{name}")]
pub(super) async fn handle_secret_patch(
    path: Path<SecretReplacePathParams>,
    patch: Json<serde_json::Map<String, serde_json::Value>>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<Secret>, Box<StatusResponse>> {
    let path = path.into_inner();
    let patch = normalize_secret_patch(patch.into_inner())?;
    resource_handlers::patch_resource::<Secret>(
        &operator,
        Some(path.namespace),
        path.name,
        patch,
        ReplaceOptions {
            preserve_status: false,
            use_client_resource_version: true,
            update_generation: true,
        },
    )
    .await
}
