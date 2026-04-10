#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;
use std::sync::OnceLock;
use std::time::Duration;

use helpers::setup::TestContext;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn scheduler_assigns_unscheduled_ship_to_available_node() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _scheduler = ctx.start_scheduler()?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-a", 4, 8 * 1024 * 1024 * 1024, &[]),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("small", 1, "1Gi"),
    )
    .await?;
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

    let scheduled = wait_for_ship(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-ship",
        Duration::from_secs(20),
        |ship| ship["spec"]["nodeName"] == "node-a",
    )
    .await?;
    assert_eq!(scheduled["spec"]["nodeName"], "node-a");

    Ok(())
}

#[tokio::test]
async fn scheduler_prefers_node_with_more_remaining_capacity() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _scheduler = ctx.start_scheduler()?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-a", 4, 8 * 1024 * 1024 * 1024, &[]),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-b", 4, 8 * 1024 * 1024 * 1024, &[]),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("small", 1, "1Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("large", 3, "6Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest_with_node(
            "test-ns",
            "existing-load",
            "large",
            "registry.example.com/demo:busy",
            "node-b",
        ),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "scored-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let scheduled = wait_for_ship(
        &client,
        &ctx.base_url,
        "test-ns",
        "scored-ship",
        Duration::from_secs(20),
        |ship| ship["spec"]["nodeName"] == "node-a",
    )
    .await?;
    assert_eq!(scheduled["spec"]["nodeName"], "node-a");

    Ok(())
}

#[tokio::test]
async fn scheduler_skips_nodes_marked_unschedulable() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _scheduler = ctx.start_scheduler()?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest_with_unschedulable("node-b", 4, 8 * 1024 * 1024 * 1024, true),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest_with_unschedulable("node-a", 4, 8 * 1024 * 1024 * 1024, false),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("small", 1, "1Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "filtered-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let scheduled = wait_for_ship(
        &client,
        &ctx.base_url,
        "test-ns",
        "filtered-ship",
        Duration::from_secs(20),
        |ship| ship["spec"]["nodeName"] == "node-a",
    )
    .await?;
    assert_eq!(scheduled["spec"]["nodeName"], "node-a");

    Ok(())
}

#[tokio::test]
async fn scheduler_leaves_ship_pending_when_no_node_can_fit() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _scheduler = ctx.start_scheduler()?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("tiny-node", 1, 512 * 1024 * 1024, &[]),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("too-large", 2, "2Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "pending-ship",
            "too-large",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    tokio::time::sleep(Duration::from_secs(3)).await;
    let ship = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships/pending-ship",
    )
    .await?;
    assert!(ship["spec"]["nodeName"].is_null());

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

fn test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

async fn wait_for_ship<F>(
    client: &Client,
    base_url: &str,
    namespace: &str,
    name: &str,
    timeout: Duration,
    predicate: F,
) -> Result<Value, DynError>
where
    F: Fn(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let ship = get_json(
            client,
            base_url,
            &format!("/api/v1/namespaces/{namespace}/ships/{name}"),
        )
        .await?;
        if predicate(&ship) {
            return Ok(ship);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("timed out waiting for ship {namespace}/{name}: {ship}").into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn create_namespace(client: &Client, base_url: &str, name: &str) -> Result<Value, DynError> {
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

async fn get_json(client: &Client, base_url: &str, path: &str) -> Result<Value, DynError> {
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
    assert_status(response, expected_status, url).await
}

async fn assert_status(
    response: Response,
    expected_status: StatusCode,
    url: &str,
) -> Result<Value, DynError> {
    let status = response.status();
    let text = response.text().await?;
    assert_eq!(
        status, expected_status,
        "unexpected status for {url}: {text}"
    );
    Ok(serde_json::from_str(&text)?)
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

fn shipclass_manifest(name: &str, cores: u64, memory: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ShipClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "cpu": {
                "architecture": "x64",
                "cores": cores
            },
            "memory": {
                "size": memory
            }
        }
    })
}

fn ship_manifest(namespace: &str, name: &str, ship_class: &str, image: &str) -> Value {
    ship_manifest_with_node_value(namespace, name, ship_class, image, Value::Null)
}

fn ship_manifest_with_node(
    namespace: &str,
    name: &str,
    ship_class: &str,
    image: &str,
    node_name: &str,
) -> Value {
    ship_manifest_with_node_value(namespace, name, ship_class, image, json!(node_name))
}

fn ship_manifest_with_node_value(
    namespace: &str,
    name: &str,
    ship_class: &str,
    image: &str,
    node_name: Value,
) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Ship",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "image": image,
            "shipClass": ship_class,
            "nodeName": node_name,
            "tolerations": []
        }
    })
}

fn node_manifest(name: &str, cpu: u64, memory: u64, taints: &[Value]) -> Value {
    node_manifest_with_unschedulable_and_taints(name, cpu, memory, false, taints)
}

fn node_manifest_with_unschedulable(
    name: &str,
    cpu: u64,
    memory: u64,
    unschedulable: bool,
) -> Value {
    node_manifest_with_unschedulable_and_taints(name, cpu, memory, unschedulable, &[])
}

fn node_manifest_with_unschedulable_and_taints(
    name: &str,
    cpu: u64,
    memory: u64,
    unschedulable: bool,
    taints: &[Value],
) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Node",
        "metadata": {
            "name": name
        },
        "spec": {
            "overcommit": {
                "cpuRatio": "1",
                "memoryRatio": "1"
            },
            "ips": ["10.0.0.10"],
            "resource": {
                "cpu": cpu,
                "memory": memory
            },
            "taints": taints,
            "unschedulable": unschedulable
        },
        "status": {
            "cniPlugins": [
                {
                    "name": "loopback",
                    "ready": true,
                    "message": "ready"
                },
                {
                    "name": "bridge",
                    "ready": true,
                    "message": "ready"
                }
            ]
        }
    })
}
