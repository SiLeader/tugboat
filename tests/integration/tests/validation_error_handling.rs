#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn rejects_invalid_resource_name() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let response = client
        .post(format!(
            "{}/api/v1/namespaces/test-ns/configmaps",
            ctx.base_url
        ))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": "Invalid Name",
                "namespace": "test-ns"
            },
            "data": {
                "mode": "dev"
            }
        }))
        .send()
        .await?;

    let body = assert_error_status(response, StatusCode::BAD_REQUEST).await?;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "BadRequest");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("Invalid ConfigMap resource"))
    );

    Ok(())
}

#[tokio::test]
async fn rejects_request_without_metadata() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let response = client
        .post(format!(
            "{}/api/v1/namespaces/test-ns/configmaps",
            ctx.base_url
        ))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "data": {
                "mode": "dev"
            }
        }))
        .send()
        .await?;

    let body = assert_error_status(response, StatusCode::BAD_REQUEST).await?;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "BadRequest");
    assert_eq!(body["message"], "metadata is required");

    Ok(())
}

#[tokio::test]
async fn rejects_namespaced_create_for_missing_namespace() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    let response = client
        .post(format!(
            "{}/api/v1/namespaces/missing-ns/ships",
            ctx.base_url
        ))
        .json(&ship_manifest(
            "missing-ns",
            "demo-ship",
            "small",
            "registry.example.com/demo:v1",
        ))
        .send()
        .await?;

    let body = assert_error_status(response, StatusCode::NOT_FOUND).await?;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "NotFound");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("missing-ns"))
    );

    Ok(())
}

#[tokio::test]
async fn rejects_invalid_json_body_with_status_response() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let response = client
        .post(format!(
            "{}/api/v1/namespaces/test-ns/configmaps",
            ctx.base_url
        ))
        .header("content-type", "application/json")
        .body("{\"apiVersion\":\"v1\",")
        .send()
        .await?;

    let body = assert_error_status(response, StatusCode::BAD_REQUEST).await?;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "BadRequest");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("Invalid JSON body"))
    );

    Ok(())
}

#[tokio::test]
async fn rejects_url_and_body_name_mismatch() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "demo-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let response = client
        .put(format!(
            "{}/api/v1/namespaces/test-ns/ships/demo-ship",
            ctx.base_url
        ))
        .json(&ship_manifest(
            "test-ns",
            "other-ship",
            "small",
            "registry.example.com/demo:v2",
        ))
        .send()
        .await?;

    let body = assert_error_status(response, StatusCode::BAD_REQUEST).await?;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "BadRequest");
    assert!(
        body["message"].as_str().is_some_and(
            |message| message.contains("metadata.name must match resource name in URL")
        )
    );

    Ok(())
}

#[tokio::test]
async fn rejects_namespace_on_cluster_scoped_resource_create() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    let response = client
        .post(format!("{}/api/v1/shipclasses", ctx.base_url))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "ShipClass",
            "metadata": {
                "name": "small",
                "namespace": "test-ns"
            },
            "spec": {
                "cpu": {
                    "architecture": "x64",
                    "cores": 2
                },
                "memory": {
                    "size": "4Gi"
                }
            }
        }))
        .send()
        .await?;

    let body = assert_error_status(response, StatusCode::BAD_REQUEST).await?;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "BadRequest");
    assert_eq!(body["message"], "metadata.namespace cannot be set");

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

async fn create_namespace(client: &Client, base_url: &str, name: &str) -> Result<Value, DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces"),
        StatusCode::CREATED,
        Some(json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": name
            }
        })),
    )
    .await
}

async fn create_resource(
    client: &Client,
    base_url: &str,
    path: &str,
    body: &Value,
) -> Result<Value, DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}{path}"),
        StatusCode::CREATED,
        Some(body.clone()),
    )
    .await
}

async fn request_json(
    client: &Client,
    method: Method,
    url: &str,
    expected_status: StatusCode,
    body: Option<Value>,
) -> Result<Value, DynError> {
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request.send().await?;
    assert_status(response, expected_status).await
}

async fn assert_status(response: Response, expected_status: StatusCode) -> Result<Value, DynError> {
    let status = response.status();
    let text = response.text().await?;
    assert_eq!(status, expected_status, "unexpected status: {text}");
    Ok(serde_json::from_str(&text)?)
}

async fn assert_error_status(
    response: Response,
    expected_status: StatusCode,
) -> Result<Value, DynError> {
    let body = assert_status(response, expected_status).await?;
    assert_eq!(body["status"], "Failure");
    assert_eq!(body["code"], expected_status.as_u16());
    Ok(body)
}

fn ship_manifest(namespace: &str, name: &str, ship_class: &str, image: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Ship",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "shipClass": ship_class,
            "image": image
        }
    })
}
