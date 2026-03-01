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

use actix_web::{get, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(components(schemas(
    tugboat_resources::manifests::core::v1::Namespace,
    tugboat_resources::manifests::core::v1::Node,
    tugboat_resources::manifests::core::v1::Ship,
    tugboat_resources::manifests::core::v1::ShipClass,
    tugboat_resources::manifests::core::v1::NetworkClass,
    tugboat_resources::manifests::core::v1::ClusterNetworkClass,
    tugboat_resources::manifests::meta::v1::ObjectMeta,
    tugboat_resources::manifests::meta::v1::TypeMeta,
    tugboat_resources::manifests::meta::v1::Time,
)))]
pub struct CoreV1ApiDoc;

#[derive(OpenApi)]
#[openapi(components(schemas(
    tugboat_resources::manifests::coordination::v1::Lease,
    tugboat_resources::manifests::meta::v1::ObjectMeta,
    tugboat_resources::manifests::meta::v1::TypeMeta,
    tugboat_resources::manifests::meta::v1::Time,
)))]
pub struct CoordinationV1ApiDoc;

#[derive(Serialize, Deserialize)]
struct DiscoveryPath {
    #[serde(rename = "serverRelativeUrl")]
    server_relative_url: String,
}

#[derive(Serialize, Deserialize)]
struct DiscoveryResponse {
    paths: HashMap<String, DiscoveryPath>,
}

#[get("/openapi/v3")]
pub async fn discovery() -> impl Responder {
    let mut paths = HashMap::new();
    paths.insert(
        "api/v1".to_string(),
        DiscoveryPath {
            server_relative_url: "/openapi/v3/api/v1".to_string(),
        },
    );
    paths.insert(
        "apis/coordination/v1".to_string(),
        DiscoveryPath {
            server_relative_url: "/openapi/v3/apis/coordination/v1".to_string(),
        },
    );

    HttpResponse::Ok().json(DiscoveryResponse { paths })
}

#[get("/openapi/v3/api/v1")]
pub async fn core_v1() -> impl Responder {
    HttpResponse::Ok().json(CoreV1ApiDoc::openapi())
}

#[get("/openapi/v3/apis/coordination/v1")]
pub async fn coordination_v1() -> impl Responder {
    HttpResponse::Ok().json(CoordinationV1ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, App};

    #[actix_web::test]
    async fn test_openapi_endpoints() {
        let app = test::init_service(
            App::new()
                .service(discovery)
                .service(core_v1)
                .service(coordination_v1),
        )
        .await;

        // Test Discovery
        let req = test::TestRequest::get().uri("/openapi/v3").to_request();
        let resp: DiscoveryResponse = test::call_and_read_body_json(&app, req).await;
        assert!(resp.paths.contains_key("api/v1"));
        assert_eq!(
            resp.paths["api/v1"].server_relative_url,
            "/openapi/v3/api/v1"
        );
        assert!(resp.paths.contains_key("apis/coordination/v1"));
        assert_eq!(
            resp.paths["apis/coordination/v1"].server_relative_url,
            "/openapi/v3/apis/coordination/v1"
        );

        // Test Core V1 Schema
        let req = test::TestRequest::get()
            .uri("/openapi/v3/api/v1")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Test Coordination V1 Schema
        let req = test::TestRequest::get()
            .uri("/openapi/v3/apis/coordination/v1")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }
}

