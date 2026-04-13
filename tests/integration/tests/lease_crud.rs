#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn lease_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/apis/coordination/v1/namespaces/test-ns/leases",
        &lease_manifest("test-ns", "scheduler-lock", "scheduler-a", 15),
    )
    .await?;
    assert_eq!(created["kind"], "Lease");
    assert_eq!(created["metadata"]["name"], "scheduler-lock");
    assert_eq!(created["spec"]["holderIdentity"], "scheduler-a");
    assert_eq!(created["spec"]["leaseDurationSeconds"], 15);

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/coordination/v1/namespaces/test-ns/leases/scheduler-lock",
    )
    .await?;
    assert_eq!(fetched["metadata"]["namespace"], "test-ns");
    assert_eq!(fetched["spec"]["strategy"], "Coordinated");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/coordination/v1/namespaces/test-ns/leases/scheduler-lock",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "renewTime": "2023-11-14T22:20:00+00:00",
                "leaseTransitions": 1
            }
        })),
    )
    .await?;
    assert_eq!(patched["spec"]["renewTime"], "2023-11-14T22:20:00+00:00");
    assert_eq!(patched["spec"]["leaseTransitions"], 1);

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/apis/coordination/v1/namespaces/test-ns/leases/scheduler-lock",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "coordination/v1",
            "kind": "Lease",
            "metadata": {
                "name": "scheduler-lock",
                "namespace": "test-ns",
                "resourceVersion": patched["metadata"]["resourceVersion"]
            },
            "spec": {
                "holderIdentity": "scheduler-b",
                "leaseDurationSeconds": 30,
                "leaseTransitions": 2,
                "preferredHolder": "scheduler-b",
                "strategy": "Coordinated",
                "acquireTime": "2023-11-14T22:20:10+00:00",
                "renewTime": "2023-11-14T22:20:20+00:00"
            }
        })),
    )
    .await?;
    assert_eq!(replaced["spec"]["holderIdentity"], "scheduler-b");
    assert_eq!(replaced["spec"]["leaseDurationSeconds"], 30);
    assert_eq!(replaced["spec"]["preferredHolder"], "scheduler-b");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/coordination/v1/namespaces/test-ns/leases/scheduler-lock",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "scheduler-lock");

    let response = client
        .get(format!(
            "{}/apis/coordination/v1/namespaces/test-ns/leases/scheduler-lock",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn lease_lists_include_namespace_and_cluster_scopes() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    for namespace in ["ns-a", "ns-b"] {
        create_namespace(&client, &ctx.base_url, namespace).await?;
    }

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/coordination/v1/namespaces/ns-a/leases",
        &lease_manifest("ns-a", "leader-a", "controller-a", 10),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/apis/coordination/v1/namespaces/ns-b/leases",
        &lease_manifest("ns-b", "leader-b", "controller-b", 20),
    )
    .await?;

    let namespace_list = get_json(
        &client,
        &ctx.base_url,
        "/apis/coordination/v1/namespaces/ns-a/leases",
    )
    .await?;
    assert_list_contains(&namespace_list, &[("ns-a", "leader-a")])?;
    assert_list_omits(&namespace_list, &[("ns-b", "leader-b")])?;

    let all_namespaces_list =
        get_json(&client, &ctx.base_url, "/apis/coordination/v1/leases").await?;
    assert_list_contains(
        &all_namespaces_list,
        &[("ns-a", "leader-a"), ("ns-b", "leader-b")],
    )?;

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
    assert_status(response, expected_status).await
}

async fn assert_status(response: Response, expected_status: StatusCode) -> Result<Value, DynError> {
    let url = response.url().clone();
    let status = response.status();
    let text = response.text().await?;
    assert_eq!(
        status, expected_status,
        "unexpected status for {url}: {text}"
    );
    Ok(serde_json::from_str(&text)?)
}

fn assert_list_contains(list: &Value, expected: &[(&str, &str)]) -> Result<(), DynError> {
    assert_eq!(list["kind"], "List");
    let items = list["items"]
        .as_array()
        .ok_or("list response is missing items")?;

    for (namespace, name) in expected {
        assert!(
            items.iter().any(|item| {
                item["metadata"]["namespace"] == *namespace && item["metadata"]["name"] == *name
            }),
            "list did not include {namespace}/{name}"
        );
    }

    Ok(())
}

fn assert_list_omits(list: &Value, omitted: &[(&str, &str)]) -> Result<(), DynError> {
    let items = list["items"]
        .as_array()
        .ok_or("list response is missing items")?;

    for (namespace, name) in omitted {
        assert!(
            !items.iter().any(|item| {
                item["metadata"]["namespace"] == *namespace && item["metadata"]["name"] == *name
            }),
            "list unexpectedly included {namespace}/{name}"
        );
    }

    Ok(())
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

fn lease_manifest(
    namespace: &str,
    name: &str,
    holder_identity: &str,
    lease_duration_seconds: i64,
) -> Value {
    json!({
        "apiVersion": "coordination/v1",
        "kind": "Lease",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "holderIdentity": holder_identity,
            "leaseDurationSeconds": lease_duration_seconds,
            "leaseTransitions": 0,
            "preferredHolder": holder_identity,
            "strategy": "Coordinated",
            "acquireTime": "2023-11-14T22:13:20+00:00",
            "renewTime": "2023-11-14T22:15:00+00:00"
        }
    })
}
