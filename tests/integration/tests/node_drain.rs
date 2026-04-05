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
async fn node_drain_marks_node_unschedulable_and_starts_ship_evacuation() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("drain-node", 4, 8 * 1024 * 1024 * 1024),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("other-node", 4, 8 * 1024 * 1024 * 1024),
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
        &ship_manifest_with_node(
            "test-ns",
            "evacuating-ship",
            "small",
            "registry.example.com/demo:v1",
            "drain-node",
        ),
    )
    .await?;

    let drain_response = request_json(
        &client,
        Method::POST,
        &format!("{}/api/v1/nodes/drain-node/drain", ctx.base_url),
        StatusCode::OK,
        None,
    )
    .await?;

    assert_eq!(drain_response["node"]["metadata"]["name"], "drain-node");
    assert_eq!(drain_response["node"]["spec"]["unschedulable"], true);
    assert_eq!(drain_response["startedCount"], 1);
    assert_eq!(drain_response["startedShips"][0], "test-ns/evacuating-ship");
    assert!(
        drain_response["warnings"]
            .as_array()
            .is_some_and(|warnings| warnings.is_empty())
    );

    let drained_node = get_json(&client, &ctx.base_url, "/api/v1/nodes/drain-node").await?;
    assert_eq!(drained_node["spec"]["unschedulable"], true);

    let ship = wait_for_ship(
        &client,
        &ctx.base_url,
        "test-ns",
        "evacuating-ship",
        Duration::from_secs(10),
        |ship| ship["spec"]["targetNodeName"] == "other-node",
    )
    .await?;
    assert_eq!(ship["spec"]["nodeName"], "drain-node");
    assert_eq!(ship["spec"]["targetNodeName"], "other-node");

    Ok(())
}

#[tokio::test]
async fn scheduler_does_not_place_new_ships_on_drained_node() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _scheduler = ctx.start_scheduler()?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("drain-node", 4, 8 * 1024 * 1024 * 1024),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("other-node", 4, 8 * 1024 * 1024 * 1024),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("small", 1, "1Gi"),
    )
    .await?;

    request_json(
        &client,
        Method::POST,
        &format!("{}/api/v1/nodes/drain-node/drain", ctx.base_url),
        StatusCode::OK,
        None,
    )
    .await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "scheduled-after-drain",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let scheduled = wait_for_ship(
        &client,
        &ctx.base_url,
        "test-ns",
        "scheduled-after-drain",
        Duration::from_secs(20),
        |ship| ship["spec"]["nodeName"] == "other-node",
    )
    .await?;
    assert_eq!(scheduled["spec"]["nodeName"], "other-node");
    assert!(scheduled["spec"]["targetNodeName"].is_null());

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

fn node_manifest(name: &str, cpu: u64, memory: u64) -> Value {
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
            "taints": [],
            "unschedulable": false
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
