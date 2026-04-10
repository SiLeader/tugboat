#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;
use reqwest::{Client, StatusCode};
use serde_json::Value;

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn discovery_endpoints_expose_expected_groups_and_resources() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;

    let api_versions = get_json(&client, &ctx.base_url, "/api").await?;
    assert_eq!(api_versions["kind"], "APIVersions");
    assert!(
        api_versions["versions"]
            .as_array()
            .is_some_and(|versions| { versions.iter().any(|version| version == "v1") })
    );

    let core_v1_resources = get_json(&client, &ctx.base_url, "/api/v1").await?;
    assert_eq!(core_v1_resources["kind"], "APIResourceList");
    assert_eq!(core_v1_resources["groupVersion"], "v1");
    let core_resources = core_v1_resources["resources"]
        .as_array()
        .ok_or("core discovery response is missing resources")?;

    for resource_name in [
        "ships",
        "namespaces",
        "nodes",
        "shipclasses",
        "persistentvolumes",
        "persistentvolumeclaims",
        "configmaps",
        "secrets",
        "storageclasses",
        "networkclasses",
        "clusternetworkclasses",
    ] {
        let resource = find_resource(core_resources, resource_name)?;
        assert_has_verbs(resource, &["create", "list", "get"])?;
    }

    let api_groups = get_json(&client, &ctx.base_url, "/apis").await?;
    assert_eq!(api_groups["kind"], "APIGroupList");
    let groups = api_groups["groups"]
        .as_array()
        .ok_or("/apis response is missing groups")?;

    let apps_group = groups
        .iter()
        .find(|group| group["name"] == "apps")
        .ok_or("apps API group is missing")?;
    assert_eq!(apps_group["preferredVersion"]["groupVersion"], "apps/v1");

    let coordination_group = groups
        .iter()
        .find(|group| group["name"] == "coordination")
        .ok_or("coordination API group is missing")?;
    assert_eq!(
        coordination_group["preferredVersion"]["groupVersion"],
        "coordination/v1"
    );

    let authorization_group = groups
        .iter()
        .find(|group| group["name"] == "authorization")
        .ok_or("authorization API group is missing")?;
    assert_eq!(
        authorization_group["preferredVersion"]["groupVersion"],
        "authorization/v1"
    );

    let apps_v1_resources = get_json(&client, &ctx.base_url, "/apis/apps/v1").await?;
    assert_eq!(apps_v1_resources["groupVersion"], "apps/v1");
    let apps_resources = apps_v1_resources["resources"]
        .as_array()
        .ok_or("apps/v1 discovery response is missing resources")?;
    for resource_name in ["deployments", "replicasets", "fleets"] {
        let resource = find_resource(apps_resources, resource_name)?;
        assert_has_verbs(resource, &["create", "list", "get"])?;
    }

    let coordination_v1_resources =
        get_json(&client, &ctx.base_url, "/apis/coordination/v1").await?;
    assert_eq!(coordination_v1_resources["groupVersion"], "coordination/v1");
    let coordination_resources = coordination_v1_resources["resources"]
        .as_array()
        .ok_or("coordination/v1 discovery response is missing resources")?;
    let lease = find_resource(coordination_resources, "leases")?;
    assert_has_verbs(lease, &["create", "list", "get", "delete"])?;

    let authorization_v1_resources =
        get_json(&client, &ctx.base_url, "/apis/authorization/v1").await?;
    assert_eq!(
        authorization_v1_resources["groupVersion"],
        "authorization/v1"
    );
    let authorization_resources = authorization_v1_resources["resources"]
        .as_array()
        .ok_or("authorization/v1 discovery response is missing resources")?;
    for resource_name in [
        "roles",
        "rolebindings",
        "clusterroles",
        "clusterrolebindings",
    ] {
        let resource = find_resource(authorization_resources, resource_name)?;
        assert_has_verbs(resource, &["create", "list", "get", "delete"])?;
    }

    let openapi_discovery = get_json(&client, &ctx.base_url, "/openapi/v3").await?;
    assert_eq!(
        openapi_discovery["paths"]["api/v1"]["serverRelativeUrl"],
        "/openapi/v3/api/v1"
    );
    assert_eq!(
        openapi_discovery["paths"]["apis/apps/v1"]["serverRelativeUrl"],
        "/openapi/v3/apis/apps/v1"
    );
    assert_eq!(
        openapi_discovery["paths"]["apis/coordination/v1"]["serverRelativeUrl"],
        "/openapi/v3/apis/coordination/v1"
    );
    assert_eq!(
        openapi_discovery["paths"]["apis/authorization/v1"]["serverRelativeUrl"],
        "/openapi/v3/apis/authorization/v1"
    );

    Ok(())
}

#[tokio::test]
async fn openapi_core_schema_includes_core_resource_definitions() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    let schema = get_json(&client, &ctx.base_url, "/openapi/v3/api/v1").await?;

    assert!(schema["paths"]["/api/v1/nodes"].is_object());
    assert!(schema["paths"]["/api/v1/shipclasses"].is_object());
    assert!(schema["paths"]["/api/v1/namespaces/{namespace}/ships"].is_object());
    assert!(schema["components"]["schemas"]["Ship"].is_object());
    assert!(schema["components"]["schemas"]["Node"].is_object());

    Ok(())
}

#[tokio::test]
async fn openapi_authorization_schema_includes_rbac_resource_definitions() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    let schema = get_json(&client, &ctx.base_url, "/openapi/v3/apis/authorization/v1").await?;

    assert!(schema["paths"]["/apis/authorization/v1/clusterroles"].is_object());
    assert!(schema["paths"]["/apis/authorization/v1/clusterrolebindings"].is_object());
    assert!(schema["paths"]["/apis/authorization/v1/namespaces/{namespace}/roles"].is_object());
    assert!(
        schema["paths"]["/apis/authorization/v1/namespaces/{namespace}/rolebindings"].is_object()
    );
    assert!(schema["components"]["schemas"]["Role"].is_object());
    assert!(schema["components"]["schemas"]["ClusterRole"].is_object());

    Ok(())
}

async fn setup_or_skip() -> Result<Option<TestContext>, DynError> {
    let Some(ctx) = TestContext::setup().await? else {
        eprintln!(
            "skipping integration test: set {} or {} (or install docker) to enable",
            "TUGBOAT_TEST_APISERVER_URL", "TUGBOAT_TEST_ETCD_ENDPOINT"
        );
        return Ok(None);
    };
    Ok(Some(ctx))
}

async fn get_json(client: &Client, base_url: &str, path: &str) -> Result<Value, DynError> {
    let response = client.get(format!("{base_url}{path}")).send().await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unexpected status for {path}"
    );
    Ok(response.json().await?)
}

fn find_resource<'a>(resources: &'a [Value], name: &str) -> Result<&'a Value, DynError> {
    resources
        .iter()
        .find(|resource| resource["name"] == name)
        .ok_or_else(|| format!("resource {name} was not present in discovery response").into())
}

fn assert_has_verbs(resource: &Value, expected: &[&str]) -> Result<(), DynError> {
    let verbs = resource["verbs"]
        .as_array()
        .ok_or("discovery resource is missing verbs")?;

    for verb in expected {
        assert!(
            verbs.iter().any(|candidate| candidate == verb),
            "resource {} is missing verb {}",
            resource["name"],
            verb
        );
    }

    Ok(())
}
