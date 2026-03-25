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

use crate::data::StatusResponse;
use crate::endpoints::resource_registry;
use actix_web::web::Path;
use actix_web::{HttpResponse, get};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiVersions {
    kind: &'static str,
    versions: Vec<&'static str>,
    server_address_by_client_cidrs: Vec<ServerAddressByClientCidr>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerAddressByClientCidr {
    client_cidr: String,
    server_address: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiGroupList {
    kind: &'static str,
    api_version: &'static str,
    groups: Vec<ApiGroup>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiGroup {
    name: String,
    versions: Vec<GroupVersionForDiscovery>,
    preferred_version: GroupVersionForDiscovery,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GroupVersionForDiscovery {
    group_version: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiResourceList {
    kind: &'static str,
    api_version: &'static str,
    group_version: String,
    resources: Vec<ApiResource>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiResource {
    name: String,
    singular_name: String,
    namespaced: bool,
    kind: String,
    verbs: Vec<&'static str>,
}

fn resource_entry(resource: resource_registry::ResourceApiDescriptor) -> ApiResource {
    ApiResource {
        name: resource.plural.to_string(),
        singular_name: resource.singular.to_string(),
        namespaced: resource.namespaced,
        kind: resource.kind.to_string(),
        verbs: resource.operations.resource_verbs(),
    }
}

fn status_subresource_entry(resource: resource_registry::ResourceApiDescriptor) -> ApiResource {
    ApiResource {
        name: format!("{}/status", resource.plural),
        singular_name: String::new(),
        namespaced: resource.namespaced,
        kind: resource.kind.to_string(),
        verbs: resource.operations.status_verbs(),
    }
}

fn expand_resources(resources: Vec<resource_registry::ResourceApiDescriptor>) -> Vec<ApiResource> {
    let mut entries = Vec::new();
    for resource in resources {
        entries.push(resource_entry(resource));
        if resource.operations.has_status_subresource() {
            entries.push(status_subresource_entry(resource));
        }
    }
    entries
}

#[derive(Deserialize, ToSchema)]
pub(super) struct GroupVersionPathParams {
    group: String,
    version: String,
}

// GET /api — list available versions for the core API group
#[utoipa::path()]
#[get("/api")]
pub(super) async fn handle_api_versions() -> HttpResponse {
    HttpResponse::Ok().json(ApiVersions {
        kind: "APIVersions",
        versions: vec!["v1"],
        server_address_by_client_cidrs: vec![],
    })
}

// GET /api/v1 — list resources in core/v1
#[utoipa::path()]
#[get("/api/v1")]
pub(super) async fn handle_api_v1_resources() -> HttpResponse {
    let resources = resource_registry::resources_for("core", "v1");
    HttpResponse::Ok().json(ApiResourceList {
        kind: "APIResourceList",
        api_version: "v1",
        group_version: "v1".to_string(),
        resources: expand_resources(resources),
    })
}

// GET /apis — list all non-core API groups
#[utoipa::path()]
#[get("/apis")]
pub(super) async fn handle_api_groups() -> HttpResponse {
    let mut grouped =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for resource in resource_registry::all_resource_apis() {
        if resource.group.is_empty() || resource.group == "core" {
            continue;
        }
        let _ = grouped
            .entry(resource.group.to_string())
            .or_default()
            .insert(resource.version.to_string());
    }

    let groups = grouped
        .into_iter()
        .map(|(group, versions)| {
            let versions = versions
                .into_iter()
                .map(|version| GroupVersionForDiscovery {
                    group_version: format!("{group}/{version}"),
                    version,
                })
                .collect::<Vec<_>>();
            let preferred_version = versions
                .first()
                .cloned()
                .unwrap_or(GroupVersionForDiscovery {
                    group_version: format!("{group}/v1"),
                    version: "v1".to_string(),
                });
            ApiGroup {
                name: group,
                versions,
                preferred_version,
            }
        })
        .collect();

    HttpResponse::Ok().json(ApiGroupList {
        kind: "APIGroupList",
        api_version: "v1",
        groups,
    })
}

// GET /apis/{group}/{version} — list resources for a specific group/version
#[utoipa::path()]
#[get("/apis/{group}/{version}")]
pub(super) async fn handle_api_group_version_resources(
    path: Path<GroupVersionPathParams>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    let resources = resource_registry::resources_for(path.group.as_str(), path.version.as_str());
    if resources.is_empty() {
        return Err(Box::new(StatusResponse::not_found(
            format!(
                "the server does not have a resource type for group \"{}\" version \"{}\"",
                path.group, path.version
            ),
            None,
        )));
    }
    Ok(HttpResponse::Ok().json(ApiResourceList {
        kind: "APIResourceList",
        api_version: "v1",
        group_version: format!("{}/{}", path.group, path.version),
        resources: expand_resources(resources),
    }))
}
