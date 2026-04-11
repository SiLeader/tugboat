#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::StatusCode;
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn namespace_can_be_created_and_read_back() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    let created = create_namespace(&client, &ctx.base_url, "test-ns").await?;

    assert_eq!(created["kind"], "Namespace");
    assert_eq!(created["apiVersion"], "v1");
    assert_eq!(created["metadata"]["name"], "test-ns");
    assert_non_empty_string(&created, "/metadata/uid")?;
    assert_non_empty_string(&created, "/metadata/creationTimestamp")?;

    let fetched = get_json(&client, &ctx.base_url, "/api/v1/namespaces/test-ns").await?;
    assert_eq!(fetched["metadata"]["name"], "test-ns");
    assert_eq!(fetched["metadata"]["uid"], created["metadata"]["uid"]);
    assert_eq!(
        fetched["metadata"]["creationTimestamp"],
        created["metadata"]["creationTimestamp"]
    );

    Ok(())
}

#[tokio::test]
async fn namespace_list_includes_created_namespaces() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    for namespace in ["ns-a", "ns-b", "ns-c"] {
        create_namespace(&client, &ctx.base_url, namespace).await?;
    }

    let listed = get_json(&client, &ctx.base_url, "/api/v1/namespaces").await?;
    assert_eq!(listed["kind"], "List");
    let items = listed["items"]
        .as_array()
        .ok_or("namespace list response is missing items")?;

    for namespace in ["ns-a", "ns-b", "ns-c"] {
        assert!(
            items
                .iter()
                .any(|item| item["metadata"]["name"] == namespace),
            "namespace list did not include {namespace}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn duplicate_namespace_creation_is_rejected() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "dup-ns").await?;

    let response = client
        .post(format!("{}/api/v1/namespaces", ctx.base_url))
        .json(&namespace_manifest("dup-ns"))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[tokio::test]
async fn reading_missing_namespace_returns_not_found() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    let response = client
        .get(format!("{}/api/v1/namespaces/nonexistent", ctx.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

async fn setup_or_skip() -> Result<Option<TestContext>, DynError> {
    let Some(ctx) = TestContext::setup().await? else {
        eprintln!(
            "skipping integration test: set TUGBOAT_TEST_APISERVER_URL or TUGBOAT_TEST_ETCD_ENDPOINT (or install docker) to enable"
        );
        return Ok(None);
    };
    Ok(Some(ctx))
}

async fn create_namespace(
    client: &SecureClient,
    base_url: &str,
    name: &str,
) -> Result<Value, DynError> {
    let response = client
        .post(format!("{base_url}/api/v1/namespaces"))
        .json(&namespace_manifest(name))
        .send()
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "unexpected status when creating namespace {name}"
    );
    Ok(response.json().await?)
}

async fn get_json(client: &SecureClient, base_url: &str, path: &str) -> Result<Value, DynError> {
    let response = client.get(format!("{base_url}{path}")).send().await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unexpected status for {path}"
    );
    Ok(response.json().await?)
}

fn namespace_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": name
        }
    })
}

fn assert_non_empty_string(value: &Value, pointer: &str) -> Result<(), DynError> {
    let actual = value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string value at {pointer}"))?;
    assert!(
        !actual.is_empty(),
        "expected non-empty string at {pointer}, got empty string"
    );
    Ok(())
}
