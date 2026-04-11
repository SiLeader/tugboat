#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn ship_list_supports_label_selectors() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest_with_labels(
            "test-ns",
            "frontend-ship",
            "small",
            "registry.example.com/demo:frontend",
            &[("app", "web"), ("tier", "frontend")],
        ),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest_with_labels(
            "test-ns",
            "backend-ship",
            "small",
            "registry.example.com/demo:backend",
            &[("app", "web"), ("tier", "backend")],
        ),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest_with_labels(
            "test-ns",
            "db-ship",
            "small",
            "registry.example.com/demo:db",
            &[("app", "db"), ("tier", "backend")],
        ),
    )
    .await?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("labelSelector", "app=web")],
    )
    .await?;
    assert_item_names(&listed, &["backend-ship", "frontend-ship"])?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("labelSelector", "tier=backend")],
    )
    .await?;
    assert_item_names(&listed, &["backend-ship", "db-ship"])?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("labelSelector", "app=web,tier=frontend")],
    )
    .await?;
    assert_item_names(&listed, &["frontend-ship"])?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("labelSelector", "app=nonexistent")],
    )
    .await?;
    assert_item_names(&listed, &[])?;

    Ok(())
}

#[tokio::test]
async fn ship_list_supports_field_selectors() -> Result<(), DynError> {
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
        "/api/v1/namespaces/ns-a/ships",
        &ship_manifest("ns-a", "alpha", "small", "registry.example.com/demo:alpha"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/ns-a/ships",
        &ship_manifest("ns-a", "bravo", "small", "registry.example.com/demo:bravo"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/ns-b/ships",
        &ship_manifest(
            "ns-b",
            "charlie",
            "small",
            "registry.example.com/demo:charlie",
        ),
    )
    .await?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/ships",
        &[("fieldSelector", "metadata.name=charlie")],
    )
    .await?;
    assert_item_names(&listed, &["charlie"])?;
    assert_eq!(listed["items"][0]["metadata"]["namespace"], "ns-b");

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/ships",
        &[("fieldSelector", "metadata.namespace=ns-a")],
    )
    .await?;
    assert_item_names(&listed, &["alpha", "bravo"])?;

    Ok(())
}

#[tokio::test]
async fn node_list_supports_label_selectors() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest_with_labels("node-a", &[("region", "apne1"), ("role", "worker")]),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest_with_labels("node-b", &[("region", "use1"), ("role", "worker")]),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest_with_labels("node-c", &[("region", "apne1"), ("role", "control-plane")]),
    )
    .await?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &[("labelSelector", "region=apne1")],
    )
    .await?;
    assert_item_names(&listed, &["node-a", "node-c"])?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &[("labelSelector", "region=apne1,role=worker")],
    )
    .await?;
    assert_item_names(&listed, &["node-a"])?;

    Ok(())
}

#[tokio::test]
async fn ship_list_all_supports_label_selectors_across_namespaces() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    for namespace in ["team-a", "team-b"] {
        create_namespace(&client, &ctx.base_url, namespace).await?;
    }

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/team-a/ships",
        &ship_manifest_with_labels(
            "team-a",
            "shared-a",
            "small",
            "registry.example.com/demo:shared-a",
            &[("track", "shared"), ("team", "a")],
        ),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/team-b/ships",
        &ship_manifest_with_labels(
            "team-b",
            "shared-b",
            "small",
            "registry.example.com/demo:shared-b",
            &[("track", "shared"), ("team", "b")],
        ),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/team-b/ships",
        &ship_manifest_with_labels(
            "team-b",
            "isolated-b",
            "small",
            "registry.example.com/demo:isolated-b",
            &[("track", "isolated"), ("team", "b")],
        ),
    )
    .await?;

    let listed = get_json_with_query(
        &client,
        &ctx.base_url,
        "/api/v1/ships",
        &[("labelSelector", "track=shared")],
    )
    .await?;
    assert_item_names(&listed, &["shared-a", "shared-b"])?;

    let namespaces = listed["items"]
        .as_array()
        .ok_or("ship list response is missing items")?
        .iter()
        .map(|item| item["metadata"]["namespace"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(namespaces.contains(&"team-a"));
    assert!(namespaces.contains(&"team-b"));

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

async fn get_json_with_query(
    client: &SecureClient,
    base_url: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Value, DynError> {
    let url = if query.is_empty() {
        format!("{base_url}{path}")
    } else {
        let query_string = query
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        format!("{base_url}{path}?{query_string}")
    };
    let response = client.get(&url).send().await?;
    assert_status(response, StatusCode::OK).await
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

fn assert_item_names(list: &Value, expected_names: &[&str]) -> Result<(), DynError> {
    assert_eq!(list["kind"], "List");
    let items = list["items"]
        .as_array()
        .ok_or("list response is missing items")?;
    let mut actual_names = items
        .iter()
        .map(|item| {
            item["metadata"]["name"]
                .as_str()
                .ok_or("item metadata.name is missing")
        })
        .collect::<Result<Vec<_>, _>>()?;
    actual_names.sort_unstable();

    let mut expected_names = expected_names.to_vec();
    expected_names.sort_unstable();
    assert_eq!(actual_names, expected_names);
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

fn ship_manifest(namespace: &str, name: &str, ship_class: &str, image: &str) -> Value {
    ship_manifest_with_labels(namespace, name, ship_class, image, &[])
}

fn ship_manifest_with_labels(
    namespace: &str,
    name: &str,
    ship_class: &str,
    image: &str,
    labels: &[(&str, &str)],
) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Ship",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": labels_to_json(labels)
        },
        "spec": {
            "image": image,
            "shipClass": ship_class
        }
    })
}

fn node_manifest_with_labels(name: &str, labels: &[(&str, &str)]) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Node",
        "metadata": {
            "name": name,
            "labels": labels_to_json(labels)
        },
        "spec": {
            "overcommit": {
                "cpuRatio": "1",
                "memoryRatio": "1"
            },
            "ips": ["10.0.0.10"],
            "resource": {
                "cpu": 4,
                "memory": 8192
            },
            "taints": [],
            "unschedulable": false
        }
    })
}

fn labels_to_json(labels: &[(&str, &str)]) -> Value {
    Value::Object(
        labels
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::String((*value).to_string())))
            .collect(),
    )
}
