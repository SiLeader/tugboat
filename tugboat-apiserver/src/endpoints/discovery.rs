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
use actix_web::web::Path;
use actix_web::{HttpResponse, get};
use serde::{Deserialize, Serialize};
use tugboat_resources::StaticResource;
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

fn resource_entry<T: StaticResource>(verbs: Vec<&'static str>) -> ApiResource {
    ApiResource {
        name: T::plural().to_string(),
        singular_name: T::singular().to_string(),
        namespaced: !T::is_cluster_scoped(),
        kind: T::kind().to_string(),
        verbs,
    }
}

fn status_subresource_entry<T: StaticResource>(verbs: Vec<&'static str>) -> ApiResource {
    ApiResource {
        name: format!("{}/status", T::plural()),
        singular_name: String::new(),
        namespaced: !T::is_cluster_scoped(),
        kind: T::kind().to_string(),
        verbs,
    }
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
    use tugboat_resources::manifests::core::v1::*;

    let default_verbs = vec!["create", "get", "list", "watch"];
    let node_verbs = vec!["create", "delete", "get", "list", "update", "watch"];

    HttpResponse::Ok().json(ApiResourceList {
        kind: "APIResourceList",
        api_version: "v1",
        group_version: "v1".to_string(),
        resources: vec![
            resource_entry::<ClusterNetworkClass>(default_verbs.clone()),
            resource_entry::<Namespace>(default_verbs.clone()),
            resource_entry::<NetworkClass>(default_verbs.clone()),
            resource_entry::<Node>(node_verbs),
            resource_entry::<Ship>(default_verbs.clone()),
            status_subresource_entry::<Ship>(vec!["patch", "update"]),
            resource_entry::<ShipClass>(default_verbs),
        ],
    })
}

// GET /apis — list all non-core API groups
#[utoipa::path()]
#[get("/apis")]
pub(super) async fn handle_api_groups() -> HttpResponse {
    let coordination = GroupVersionForDiscovery {
        group_version: "coordination/v1".to_string(),
        version: "v1".to_string(),
    };

    HttpResponse::Ok().json(ApiGroupList {
        kind: "APIGroupList",
        api_version: "v1",
        groups: vec![ApiGroup {
            name: "coordination".to_string(),
            versions: vec![coordination.clone()],
            preferred_version: coordination,
        }],
    })
}

// GET /apis/{group}/{version} — list resources for a specific group/version
#[utoipa::path()]
#[get("/apis/{group}/{version}")]
pub(super) async fn handle_api_group_version_resources(
    path: Path<GroupVersionPathParams>,
) -> Result<HttpResponse, StatusResponse> {
    use tugboat_resources::manifests::coordination::v1::*;

    let path = path.into_inner();
    let default_verbs = vec!["create", "get", "list", "watch"];

    match (path.group.as_str(), path.version.as_str()) {
        ("coordination", "v1") => Ok(HttpResponse::Ok().json(ApiResourceList {
            kind: "APIResourceList",
            api_version: "v1",
            group_version: "coordination/v1".to_string(),
            resources: vec![resource_entry::<Lease>(default_verbs)],
        })),
        _ => Err(StatusResponse::not_found(
            format!(
                "the server does not have a resource type for group \"{}\" version \"{}\"",
                path.group, path.version
            ),
            None,
        )),
    }
}
