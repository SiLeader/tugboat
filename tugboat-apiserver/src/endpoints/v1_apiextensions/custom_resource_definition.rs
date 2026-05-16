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
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::apiextensions::v1::CustomResourceDefinition;
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_resources::validators::Validatable;

async fn ensure_immutable_spec(
    operator: &ApiOperator,
    name: &str,
    new_scope: Option<&str>,
    new_version_name: Option<&str>,
) -> Result<(), Box<StatusResponse>> {
    if new_scope.is_none() && new_version_name.is_none() {
        return Ok(());
    }
    let current = operator
        .store
        .get::<CustomResourceDefinition>(None, name)
        .await
        .map_err(|err| Box::new(err.into()))?;
    let Some(current) = current else {
        return Ok(());
    };

    if let Some(new_scope) = new_scope {
        let Some(current_scope) = current.data.spec.as_ref().map(|spec| spec.scope.as_str()) else {
            return Ok(());
        };
        if current_scope != new_scope {
            return Err(Box::new(StatusResponse::invalid(
                format!(
                    "CustomResourceDefinition spec.scope cannot be changed once set (current: \"{current_scope}\", requested: \"{new_scope}\")"
                ),
                Some(serde_json::json!({
                    "name": name,
                    "currentScope": current_scope,
                    "requestedScope": new_scope,
                })),
            )));
        }
    }

    if let Some(new_version_name) = new_version_name {
        let Some(current_version_name) = current
            .data
            .spec
            .as_ref()
            .and_then(|spec| spec.versions.first())
            .map(|version| version.name.as_str())
        else {
            return Ok(());
        };
        if current_version_name != new_version_name {
            return Err(Box::new(StatusResponse::invalid(
                format!(
                    "CustomResourceDefinition spec.versions[0].name cannot be changed once set (current: \"{current_version_name}\", requested: \"{new_version_name}\")"
                ),
                Some(serde_json::json!({
                    "name": name,
                    "currentVersion": current_version_name,
                    "requestedVersion": new_version_name,
                })),
            )));
        }
    }

    Ok(())
}

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
    let name = path.into_inner().name;
    let current = operator
        .store
        .get::<CustomResourceDefinition>(None, &name)
        .await
        .map_err(|err| Box::new(err.into()))?;
    let Some(current) = current else {
        return Err(Box::new(StatusResponse::not_found(
            "CustomResourceDefinition not found",
            Some(serde_json::json!({ "name": name })),
        )));
    };
    let current = current.apply_revision();

    if current.has_finalizers() {
        let mut pending_delete = current.clone();
        pending_delete.mark_for_deletion(Time::now());
        let pending_delete = if current != pending_delete {
            operator
                .store
                .put(pending_delete)
                .await
                .map_err(|err| Box::new(err.into()))?
                .apply_revision()
        } else {
            pending_delete
        };

        return Ok(ReadResponse::new(pending_delete));
    }

    delete_custom_resources_for_crd(&operator, &current).await?;
    let deleted = operator
        .store
        .delete::<CustomResourceDefinition>(None, &name)
        .await
        .map_err(|err| Box::new(err.into()))?;
    match deleted {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(Box::new(StatusResponse::not_found(
            "CustomResourceDefinition not found",
            Some(serde_json::json!({ "name": name })),
        ))),
    }
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
    let name = path.into_inner().name;
    validate_custom_resource_definition_schema(&replacement)?;
    let new_scope = replacement.spec.as_ref().map(|spec| spec.scope.as_str());
    let new_version_name = replacement
        .spec
        .as_ref()
        .and_then(|spec| spec.versions.first())
        .map(|version| version.name.as_str());
    ensure_immutable_spec(&operator, &name, new_scope, new_version_name).await?;
    resource_handlers::replace_resource::<CustomResourceDefinition>(
        &operator,
        None,
        name,
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
    let name = path.into_inner().name;
    let patch_inner = patch.into_inner();
    let new_scope = patch_inner
        .get("spec")
        .and_then(|spec| spec.get("scope"))
        .and_then(|scope| scope.as_str());
    let new_version_name = patch_inner
        .get("spec")
        .and_then(|spec| spec.get("versions"))
        .and_then(|versions| versions.as_array())
        .and_then(|versions| versions.first())
        .and_then(|version| version.get("name"))
        .and_then(|name| name.as_str());
    ensure_immutable_spec(&operator, &name, new_scope, new_version_name).await?;
    resource_handlers::patch_resource_with_validation::<CustomResourceDefinition, _>(
        &operator,
        None,
        name,
        patch_inner,
        ReplaceOptions {
            preserve_status: true,
            use_client_resource_version: false,
            update_generation: true,
        },
        validate_custom_resource_definition_schema,
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

async fn delete_custom_resources_for_crd(
    operator: &ApiOperator,
    crd: &CustomResourceDefinition,
) -> Result<(), Box<StatusResponse>> {
    let Some(spec) = crd.spec.as_ref() else {
        return Ok(());
    };
    let Some(names) = spec.names.as_ref() else {
        return Ok(());
    };

    operator
        .store
        .delete_custom_collection(&spec.group, &names.plural)
        .await
        .map(|_| ())
        .map_err(|err| Box::new(err.into()))
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

#[cfg(test)]
mod tests {
    use super::validate_custom_resource_definition_schema;
    use tugboat_resources::manifests::apiextensions::v1::{
        CustomResourceDefinition, CustomResourceDefinitionNames, CustomResourceDefinitionSpec,
        CustomResourceDefinitionVersion, CustomResourceValidation,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    fn crd_with_schema(schema: &str) -> CustomResourceDefinition {
        CustomResourceDefinition {
            object_meta: Some(ObjectMeta {
                name: Some("widgets.example.com".to_string()),
                ..Default::default()
            }),
            spec: Some(CustomResourceDefinitionSpec {
                group: "example.com".to_string(),
                names: Some(CustomResourceDefinitionNames {
                    plural: "widgets".to_string(),
                    singular: "widget".to_string(),
                    kind: "Widget".to_string(),
                    list_kind: "WidgetList".to_string(),
                }),
                scope: "Namespaced".to_string(),
                versions: vec![CustomResourceDefinitionVersion {
                    name: "v1".to_string(),
                    served: true,
                    storage: true,
                    schema: Some(CustomResourceValidation {
                        open_api_v3_schema: schema.to_string(),
                    }),
                    subresources: None,
                }],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn validate_custom_resource_definition_schema_rejects_invalid_json_schema() {
        let crd = crd_with_schema(r#"{"type":"not-a-json-schema-type"}"#);

        assert!(validate_custom_resource_definition_schema(&crd).is_err());
    }
}
