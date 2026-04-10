#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn ship_assigns_resource_version_and_updates_it_on_patch() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "versioned-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let initial_rv = string_field(&created, &["metadata", "resourceVersion"])?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/versioned-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "example.com/owner": "resource-version-test"
                }
            }
        })),
    )
    .await?;
    let patched_rv = string_field(&patched, &["metadata", "resourceVersion"])?;

    assert_ne!(initial_rv, patched_rv);

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships/versioned-ship",
    )
    .await?;
    assert_eq!(
        fetched["metadata"]["annotations"]["example.com/owner"],
        "resource-version-test"
    );
    assert_eq!(
        fetched["metadata"]["resourceVersion"],
        patched["metadata"]["resourceVersion"]
    );

    Ok(())
}

#[tokio::test]
async fn ship_rejects_replace_with_stale_resource_version() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "conflict-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let stale_rv = string_field(&created, &["metadata", "resourceVersion"])?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/conflict-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "example.com/revision": "current"
                }
            }
        })),
    )
    .await?;
    assert_ne!(
        stale_rv,
        string_field(&patched, &["metadata", "resourceVersion"])?,
    );

    let response = client
        .put(format!(
            "{}/api/v1/namespaces/test-ns/ships/conflict-ship",
            ctx.base_url
        ))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "Ship",
            "metadata": {
                "name": "conflict-ship",
                "namespace": "test-ns",
                "resourceVersion": stale_rv
            },
            "spec": {
                "shipClass": "medium",
                "image": "registry.example.com/demo:v2"
            }
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value = response.json().await?;
    assert_eq!(body["reason"], "Conflict");
    assert_eq!(body["message"], "Resources are conflicted");

    Ok(())
}

#[tokio::test]
async fn ship_spec_updates_bump_generation() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "generation-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let initial_generation = integer_field(&created, &["metadata", "generation"])?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/generation-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "image": "registry.example.com/demo:v2"
            }
        })),
    )
    .await?;
    let updated_generation = integer_field(&patched, &["metadata", "generation"])?;

    assert!(
        updated_generation > initial_generation,
        "expected generation to increase: before={initial_generation}, after={updated_generation}"
    );

    Ok(())
}

#[tokio::test]
async fn ship_status_updates_do_not_bump_generation() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "status-generation-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let initial_generation = integer_field(&created, &["metadata", "generation"])?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/status-generation-ship/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "status": {
                "conditions": [
                    {
                        "status": "Ready",
                        "message": "running"
                    }
                ]
            }
        })),
    )
    .await?;
    let updated_generation = integer_field(&patched, &["metadata", "generation"])?;

    assert_eq!(updated_generation, initial_generation);

    Ok(())
}

#[tokio::test]
async fn recreating_ship_with_same_name_gets_new_uid() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "recreated-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let first_uid = string_field(&created, &["metadata", "uid"])?;

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/recreated-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    let recreated = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "recreated-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let second_uid = string_field(&recreated, &["metadata", "uid"])?;

    assert_ne!(first_uid, second_uid);

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

async fn create_namespace(
    client: &SecureClient,
    base_url: &str,
    name: &str,
) -> Result<Value, DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces"),
        StatusCode::CREATED,
        Some(namespace_manifest(name)),
    )
    .await
}

async fn create_resource(
    client: &SecureClient,
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

async fn get_json(client: &SecureClient, base_url: &str, path: &str) -> Result<Value, DynError> {
    request_json(
        client,
        Method::GET,
        &format!("{base_url}{path}"),
        StatusCode::OK,
        None,
    )
    .await
}

async fn request_json(
    client: &SecureClient,
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
    parse_json_response(response, expected_status).await
}

async fn parse_json_response(
    response: Response,
    expected_status: StatusCode,
) -> Result<Value, DynError> {
    let status = response.status();
    let text = response.text().await?;
    assert_eq!(
        status, expected_status,
        "unexpected status {status}; body: {text}"
    );

    Ok(serde_json::from_str(&text)?)
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

fn namespace_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": name
        }
    })
}

fn string_field(value: &Value, path: &[&str]) -> Result<String, DynError> {
    field_at(value, path)?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("expected string at {}", path.join(".")).into())
}

fn integer_field(value: &Value, path: &[&str]) -> Result<i64, DynError> {
    field_at(value, path)?
        .as_i64()
        .ok_or_else(|| format!("expected integer at {}", path.join(".")).into())
}

fn field_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a Value, DynError> {
    let mut current = value;
    for segment in path {
        current = current
            .get(*segment)
            .ok_or_else(|| format!("missing field {}", path.join(".")))?;
    }
    Ok(current)
}
