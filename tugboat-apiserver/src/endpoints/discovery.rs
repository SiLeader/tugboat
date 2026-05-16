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

use crate::crd_registry::{CrdEntry, CrdRegistry, CrdScope};
use crate::data::StatusResponse;
use crate::endpoints::resource_registry;
use actix_web::web::{Data, Path};
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

const CUSTOM_RESOURCE_VERBS: [&str; 7] = [
    "create", "delete", "get", "list", "patch", "update", "watch",
];
const CUSTOM_RESOURCE_STATUS_VERBS: [&str; 2] = ["patch", "update"];

fn resource_entry(resource: resource_registry::ResourceApiDescriptor) -> ApiResource {
    ApiResource {
        name: resource.plural.to_string(),
        singular_name: resource.singular.to_string(),
        namespaced: resource.namespaced(),
        kind: resource.kind.to_string(),
        verbs: resource.operations.resource_verbs(),
    }
}

fn status_subresource_entry(resource: resource_registry::ResourceApiDescriptor) -> ApiResource {
    ApiResource {
        name: format!("{}/status", resource.plural),
        singular_name: String::new(),
        namespaced: resource.namespaced(),
        kind: resource.kind.to_string(),
        verbs: resource.operations.status_verbs(),
    }
}

fn expand_resources(resources: &[resource_registry::ResourceApiDescriptor]) -> Vec<ApiResource> {
    let mut entries = Vec::new();
    for resource in resources {
        entries.push(resource_entry(*resource));
        if resource.operations.has_status_subresource() {
            entries.push(status_subresource_entry(*resource));
        }
    }
    entries
}

fn custom_resource_entry(entry: &CrdEntry) -> ApiResource {
    ApiResource {
        name: entry.plural.clone(),
        singular_name: entry.singular.clone(),
        namespaced: entry.scope == CrdScope::Namespaced,
        kind: entry.kind.clone(),
        verbs: CUSTOM_RESOURCE_VERBS.to_vec(),
    }
}

fn custom_status_subresource_entry(entry: &CrdEntry) -> ApiResource {
    ApiResource {
        name: format!("{}/status", entry.plural),
        singular_name: String::new(),
        namespaced: entry.scope == CrdScope::Namespaced,
        kind: entry.kind.clone(),
        verbs: CUSTOM_RESOURCE_STATUS_VERBS.to_vec(),
    }
}

fn expand_custom_resources(mut entries: Vec<CrdEntry>) -> Vec<ApiResource> {
    entries.sort_by(|left, right| {
        (
            left.group.as_str(),
            left.version.name.as_str(),
            left.plural.as_str(),
        )
            .cmp(&(
                right.group.as_str(),
                right.version.name.as_str(),
                right.plural.as_str(),
            ))
    });
    let mut resources = Vec::new();
    for entry in entries.into_iter().filter(|entry| entry.version.served) {
        resources.push(custom_resource_entry(&entry));
        if entry.version.status_subresource {
            resources.push(custom_status_subresource_entry(&entry));
        }
    }
    resources
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
        resources: expand_resources(&resources),
    })
}

// GET /apis — list all non-core API groups
#[utoipa::path()]
#[get("/apis")]
pub(super) async fn handle_api_groups(registry: Data<CrdRegistry>) -> HttpResponse {
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
    for entry in registry.list_all() {
        if entry.version.served {
            let _ = grouped
                .entry(entry.group)
                .or_default()
                .insert(entry.version.name);
        }
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
    registry: Data<CrdRegistry>,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let path = path.into_inner();
    let resources = resource_registry::resources_for(path.group.as_str(), path.version.as_str());
    let custom_resources = registry
        .list_all()
        .into_iter()
        .filter(|entry| entry.group == path.group && entry.version.name == path.version)
        .collect::<Vec<_>>();
    if resources.is_empty() {
        let custom = expand_custom_resources(custom_resources);
        if !custom.is_empty() {
            return Ok(HttpResponse::Ok().json(ApiResourceList {
                kind: "APIResourceList",
                api_version: "v1",
                group_version: format!("{}/{}", path.group, path.version),
                resources: custom,
            }));
        }
        return Err(Box::new(StatusResponse::not_found(
            format!(
                "the server does not have a resource type for group \"{}\" version \"{}\"",
                path.group, path.version
            ),
            None,
        )));
    }
    let mut expanded = expand_resources(&resources);
    expanded.extend(expand_custom_resources(custom_resources));
    Ok(HttpResponse::Ok().json(ApiResourceList {
        kind: "APIResourceList",
        api_version: "v1",
        group_version: format!("{}/{}", path.group, path.version),
        resources: expanded,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd_registry::{CrdEntry, CrdScope, CrdVersionInfo};
    use actix_web::{App, test as actix_test};
    use serde_json::Value;

    #[test]
    fn discovery_entries_are_derived_from_resource_descriptors() {
        let entries = expand_resources(resource_registry::all_resource_apis());

        for resource in resource_registry::all_resource_apis() {
            let entry = entries
                .iter()
                .find(|entry| entry.name == resource.plural)
                .expect("resource discovery entry should exist");
            assert_eq!(entry.singular_name, resource.singular);
            assert_eq!(entry.namespaced, resource.namespaced());
            assert_eq!(entry.kind, resource.kind);
            assert_eq!(entry.verbs, resource.operations.resource_verbs());

            let status_entry = entries
                .iter()
                .find(|entry| entry.name == format!("{}/status", resource.plural));
            if resource.operations.has_status_subresource() {
                let status_entry = status_entry.expect("status discovery entry should exist");
                assert_eq!(status_entry.namespaced, resource.namespaced());
                assert_eq!(status_entry.kind, resource.kind);
                assert_eq!(status_entry.verbs, resource.operations.status_verbs());
            } else {
                assert!(status_entry.is_none());
            }
        }
    }

    #[actix_web::test]
    async fn core_discovery_includes_configmap_with_watch_verb() {
        let app = actix_test::init_service(App::new().service(handle_api_v1_resources)).await;

        let req = actix_test::TestRequest::get().uri("/api/v1").to_request();
        let resp: Value = actix_test::call_and_read_body_json(&app, req).await;
        let resources = resp["resources"].as_array().unwrap();
        let configmaps = resources
            .iter()
            .find(|resource| resource["name"] == "configmaps")
            .unwrap();

        assert_eq!(configmaps["kind"], "ConfigMap");
        assert_eq!(configmaps["namespaced"], true);
        assert!(
            configmaps["verbs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|verb| verb == "watch")
        );
    }

    #[actix_web::test]
    async fn snapshot_discovery_lists_resources_and_status_subresources() {
        let app = actix_test::init_service(
            App::new()
                .app_data(empty_registry_data())
                .service(handle_api_group_version_resources),
        )
        .await;

        let req = actix_test::TestRequest::get()
            .uri("/apis/snapshot/v1")
            .to_request();
        let resp: Value = actix_test::call_and_read_body_json(&app, req).await;
        let resources = resp["resources"].as_array().unwrap();

        for (name, kind, namespaced) in [
            ("volumesnapshots", "VolumeSnapshot", true),
            ("volumesnapshotcontents", "VolumeSnapshotContent", false),
            ("volumesnapshotclasses", "VolumeSnapshotClass", false),
            ("shipsnapshots", "ShipSnapshot", true),
        ] {
            let resource = resources
                .iter()
                .find(|resource| resource["name"] == name)
                .unwrap_or_else(|| panic!("missing resource {name}"));
            assert_eq!(resource["kind"], kind);
            assert_eq!(resource["namespaced"], namespaced);
            for verb in [
                "create", "delete", "get", "list", "patch", "update", "watch",
            ] {
                assert!(
                    resource["verbs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v == verb),
                    "{name} missing {verb}"
                );
            }

            let status_name = format!("{name}/status");
            let status = resources
                .iter()
                .find(|resource| resource["name"] == status_name)
                .unwrap_or_else(|| panic!("missing resource {status_name}"));
            assert_eq!(status["kind"], kind);
            assert_eq!(status["namespaced"], namespaced);
            assert_eq!(status["verbs"], serde_json::json!(["patch", "update"]));
        }
    }

    #[actix_web::test]
    async fn api_groups_include_crd_groups() {
        let registry = registry_with_crd(CrdScope::Namespaced, true);
        let app = actix_test::init_service(
            App::new()
                .app_data(Data::new(registry))
                .service(handle_api_groups),
        )
        .await;

        let req = actix_test::TestRequest::get().uri("/apis").to_request();
        let resp: Value = actix_test::call_and_read_body_json(&app, req).await;
        let groups = resp["groups"].as_array().unwrap();
        let example = groups
            .iter()
            .find(|group| group["name"] == "example.com")
            .expect("CRD group should be listed");

        assert_eq!(
            example["preferredVersion"]["groupVersion"],
            "example.com/v1"
        );
    }

    #[actix_web::test]
    async fn group_version_discovery_includes_crd_resources_and_status() {
        let registry = registry_with_crd(CrdScope::Namespaced, true);
        let app = actix_test::init_service(
            App::new()
                .app_data(Data::new(registry))
                .service(handle_api_group_version_resources),
        )
        .await;

        let req = actix_test::TestRequest::get()
            .uri("/apis/example.com/v1")
            .to_request();
        let resp: Value = actix_test::call_and_read_body_json(&app, req).await;
        let resources = resp["resources"].as_array().unwrap();
        let widget = resources
            .iter()
            .find(|resource| resource["name"] == "widgets")
            .expect("CRD resource should be listed");
        let status = resources
            .iter()
            .find(|resource| resource["name"] == "widgets/status")
            .expect("CRD status subresource should be listed");

        assert_eq!(resp["groupVersion"], "example.com/v1");
        assert_eq!(widget["singularName"], "widget");
        assert_eq!(widget["namespaced"], true);
        assert_eq!(
            widget["verbs"],
            serde_json::json!([
                "create", "delete", "get", "list", "patch", "update", "watch"
            ])
        );
        assert_eq!(status["kind"], "Widget");
        assert_eq!(status["verbs"], serde_json::json!(["patch", "update"]));
    }

    fn empty_registry_data() -> Data<CrdRegistry> {
        Data::new(CrdRegistry::default())
    }

    fn registry_with_crd(scope: CrdScope, status_subresource: bool) -> CrdRegistry {
        let registry = CrdRegistry::default();
        registry
            .upsert(CrdEntry {
                group: "example.com".to_string(),
                plural: "widgets".to_string(),
                singular: "widget".to_string(),
                kind: "Widget".to_string(),
                list_kind: "WidgetList".to_string(),
                scope,
                version: CrdVersionInfo {
                    name: "v1".to_string(),
                    served: true,
                    storage: true,
                    schema_json: None,
                    compiled_schema: None,
                    status_subresource,
                },
            })
            .expect("CRD registry update should succeed");
        registry
    }
}
