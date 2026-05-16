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
use crate::endpoints::selector::Selector;
use crate::endpoints::v1_apiextensions::openapi_apiextensions_v1;
use crate::endpoints::v1_apps::openapi_apps_v1;
use crate::endpoints::v1_authorization::openapi_authorization_v1;
use crate::endpoints::v1_coordination::openapi_coordination_v1;
use crate::endpoints::v1_core::openapi_core_v1;
use crate::endpoints::v1_snapshot::openapi_snapshot_v1;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_actix_web::service_config::ServiceConfig;

mod custom_resources;
mod discovery;
mod openid;
mod resource_handlers;
pub(crate) mod resource_registry;
mod selector;
mod utils;
mod v1_apiextensions;
mod v1_apps;
mod v1_authorization;
mod v1_coordination;
mod v1_core;
mod v1_snapshot;
mod watch_utils;

pub mod openapi;

pub fn register_openapi_endpoints(config: &mut actix_web::web::ServiceConfig) {
    config
        .service(openapi::discovery)
        .service(openapi_apiextensions_v1)
        .service(openapi_authorization_v1)
        .service(openapi_apps_v1)
        .service(openapi_core_v1)
        .service(openapi_coordination_v1)
        .service(openapi_snapshot_v1)
        .service(openid::jwks)
        .service(openid::openid_configuration);
}

pub(super) fn register_endpoints(config: &mut ServiceConfig) {
    resource_registry::register_resource_apis(config);
    config
        .service(discovery::handle_api_versions)
        .service(discovery::handle_api_v1_resources)
        .service(discovery::handle_api_groups)
        .service(discovery::handle_api_group_version_resources);
}

#[derive(Deserialize, ToSchema)]
struct NamespacedPathParams {
    namespace: String,
}

/// Shared path params for namespaced resources that require both namespace and name.
#[derive(Deserialize, ToSchema)]
struct NamespacedNamePathParams {
    namespace: String,
    name: String,
}

/// Shared path params for cluster-scoped resources that require only a name.
#[derive(Deserialize, ToSchema)]
struct ClusterNamePathParams {
    name: String,
}

#[derive(Deserialize, Copy, Clone)]
enum WatchOption {
    #[serde(rename = "true", alias = "True")]
    True, // default watch mode
    #[serde(rename = "ndJson", alias = "NdJson")]
    NdJson,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    watch: Option<WatchOption>,
    resource_version: Option<String>,
    field_selector: Option<String>,
    label_selector: Option<String>,
}

impl ListQuery {
    fn to_field_selector(&self) -> Result<Option<Vec<Selector>>, Box<StatusResponse>> {
        if let Some(field_selector) = &self.field_selector {
            Selector::try_parse(field_selector).map(Some)
        } else {
            Ok(None)
        }
    }

    fn to_label_selector(&self) -> Result<Option<Vec<Selector>>, Box<StatusResponse>> {
        if let Some(label_selector) = &self.label_selector {
            Selector::try_parse(label_selector).map(Some)
        } else {
            Ok(None)
        }
    }
}
