#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;
use std::sync::OnceLock;
use std::time::Duration;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn replicaset_controller_creates_ships_for_desired_replicas() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets",
        &replicaset_manifest("test-ns", "demo-rs", 3, "example.com/images/demo:v1"),
    )
    .await?;
    let rs_uid = created["metadata"]["uid"]
        .as_str()
        .ok_or("replicaset uid missing")?;

    let ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-rs",
        3,
        Duration::from_secs(20),
    )
    .await?;

    for ship in ships {
        assert_eq!(ship["metadata"]["namespace"], "test-ns");
        assert_eq!(ship["spec"]["image"], "example.com/images/demo:v1");
        assert_eq!(ship["spec"]["shipClass"], "standard");
        assert_eq!(ship["metadata"]["labels"]["app"], "demo");
        assert_eq!(ship["metadata"]["labels"]["tier"], "backend");
        assert!(
            ship["metadata"]["ownerReferences"]
                .as_array()
                .is_some_and(|owners| owners.iter().any(|owner| {
                    owner["kind"] == "ReplicaSet"
                        && owner["name"] == "demo-rs"
                        && owner["uid"] == rs_uid
                        && owner["controller"] == true
                }))
        );
    }

    Ok(())
}

#[tokio::test]
async fn replicaset_controller_scales_up_and_down() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets",
        &replicaset_manifest("test-ns", "scale-rs", 2, "example.com/images/demo:v1"),
    )
    .await?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "scale-rs",
        2,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/replicasets/scale-rs",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "replicas": 5
            }
        })),
    )
    .await?;

    let ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "scale-rs",
        5,
        Duration::from_secs(20),
    )
    .await?;
    for ship in &ships {
        assert_eq!(ship["spec"]["image"], "example.com/images/demo:v1");
        assert_eq!(ship["metadata"]["labels"]["app"], "demo");
    }

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/replicasets/scale-rs",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "replicas": 1
            }
        })),
    )
    .await?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "scale-rs",
        1,
        Duration::from_secs(20),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn replicaset_controller_recreates_deleted_ship() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets",
        &replicaset_manifest("test-ns", "heal-rs", 2, "example.com/images/demo:v1"),
    )
    .await?;

    let initial_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "heal-rs",
        2,
        Duration::from_secs(20),
    )
    .await?;
    let deleted_name = initial_ships[0]["metadata"]["name"]
        .as_str()
        .ok_or("ship name missing")?
        .to_string();

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/{deleted_name}",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    let healed_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "heal-rs",
        2,
        Duration::from_secs(20),
    )
    .await?;
    assert!(
        healed_ships
            .iter()
            .filter_map(|ship| ship["metadata"]["name"].as_str())
            .any(|name| name != deleted_name),
        "controller did not create a replacement ship"
    );

    Ok(())
}

#[tokio::test]
async fn replicaset_controller_deletes_owned_ships_when_replicaset_is_deleted()
-> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets",
        &replicaset_manifest("test-ns", "delete-rs", 2, "example.com/images/demo:v1"),
    )
    .await?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "delete-rs",
        2,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/replicasets/delete-rs",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "delete-rs",
        0,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::GET,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/replicasets/delete-rs",
            ctx.base_url
        ),
        StatusCode::NOT_FOUND,
        None,
    )
    .await?;

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

async fn wait_for_owned_ship_count(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    owner_name: &str,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<Value>, DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let ships = list_owned_ships(client, base_url, namespace, owner_name).await?;
        if ships.len() == expected {
            return Ok(ships);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {expected} ships owned by {namespace}/{owner_name}: {ships:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn list_owned_ships(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    owner_name: &str,
) -> Result<Vec<Value>, DynError> {
    let list = get_json(
        client,
        base_url,
        &format!("/api/v1/namespaces/{namespace}/ships"),
    )
    .await?;
    let items = list["items"]
        .as_array()
        .ok_or("ship list response is missing items")?;

    Ok(items
        .iter()
        .filter(|ship| {
            ship["metadata"]["ownerReferences"]
                .as_array()
                .is_some_and(|owners| {
                    owners.iter().any(|owner| {
                        owner["kind"] == "ReplicaSet"
                            && owner["name"] == owner_name
                            && owner["controller"] == true
                    })
                })
        })
        .cloned()
        .collect())
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
    let status = response.status();
    let text = response.text().await?;
    assert_eq!(status, expected_status, "unexpected status: {text}");
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

fn replicaset_manifest(namespace: &str, name: &str, replicas: i32, image: &str) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "ReplicaSet",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "replicas": replicas,
            "selector": {
                "app": "demo"
            },
            "shipTemplate": {
                "metadata": {
                    "labels": {
                        "app": "demo",
                        "tier": "backend"
                    }
                },
                "spec": {
                    "image": image,
                    "shipClass": "standard"
                }
            }
        }
    })
}
