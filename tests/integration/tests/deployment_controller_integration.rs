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
async fn deployment_controller_creates_owned_replicaset() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    let deployment = create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/deployments",
        &deployment_manifest(
            "test-ns",
            "demo-deployment",
            "demo-create",
            2,
            "example.com/images/demo:v1",
        ),
    )
    .await?;
    let deployment_uid = string_field(&deployment, &["metadata", "uid"])?;

    let replicasets = wait_for_owned_replicaset_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-deployment",
        1,
        Duration::from_secs(20),
    )
    .await?;
    let replicaset = &replicasets[0];

    assert_eq!(replicaset["spec"]["replicas"], 2);
    assert_eq!(
        replicaset["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/demo:v1"
    );
    assert!(
        replicaset["metadata"]["ownerReferences"]
            .as_array()
            .is_some_and(|owners| owners.iter().any(|owner| {
                owner["kind"] == "Deployment"
                    && owner["name"] == "demo-deployment"
                    && owner["uid"] == deployment_uid
                    && owner["controller"] == true
            }))
    );

    Ok(())
}

#[tokio::test]
async fn deployment_controller_scales_managed_replicaset() -> Result<(), DynError> {
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
        "/apis/apps/v1/namespaces/test-ns/deployments",
        &deployment_manifest(
            "test-ns",
            "scale-deployment",
            "demo-scale",
            2,
            "example.com/images/demo:v1",
        ),
    )
    .await?;

    wait_for_owned_replicaset_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "scale-deployment",
        1,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/scale-deployment",
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

    let scaled = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "scale-deployment",
        Duration::from_secs(20),
        |rs| rs["spec"]["replicas"] == 5,
    )
    .await?;

    assert_eq!(
        scaled["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/demo:v1"
    );

    Ok(())
}

#[tokio::test]
async fn deployment_controller_performs_rolling_update() -> Result<(), DynError> {
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
        "/apis/apps/v1/namespaces/test-ns/deployments",
        &deployment_manifest(
            "test-ns",
            "rollout-deployment",
            "demo-rollout",
            2,
            "example.com/images/demo:v1",
        ),
    )
    .await?;

    let initial_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "rollout-deployment",
        Duration::from_secs(20),
        |rs| rs["spec"]["replicas"] == 2,
    )
    .await?;
    let initial_rs_name = string_field(&initial_rs, &["metadata", "name"])?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    mark_labeled_ships_running(&client, &ctx.base_url, "test-ns", "app", "demo-rollout").await?;
    wait_for_replicaset_ready_replicas(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/rollout-deployment",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "shipTemplate": {
                    "metadata": {
                        "labels": {
                            "app": "demo-rollout"
                        }
                    },
                    "spec": {
                        "image": "example.com/images/demo:v2",
                        "shipClass": "standard"
                    }
                }
            }
        })),
    )
    .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let (old_rs, new_rs) = loop {
        mark_labeled_ships_running(&client, &ctx.base_url, "test-ns", "app", "demo-rollout")
            .await?;

        let replicasets =
            list_owned_replicasets(&client, &ctx.base_url, "test-ns", "rollout-deployment").await?;
        let old_rs = replicasets
            .iter()
            .find(|rs| {
                rs["metadata"]["name"] == initial_rs_name
                    && rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/demo:v1"
            })
            .cloned();
        let new_rs = replicasets
            .iter()
            .find(|rs| {
                rs["metadata"]["name"] != initial_rs_name
                    && rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/demo:v2"
            })
            .cloned();

        if let (Some(old_rs), Some(new_rs)) = (old_rs, new_rs) {
            let old_zero = old_rs["spec"]["replicas"] == 0;
            let new_full = new_rs["spec"]["replicas"] == 2;
            let new_ready = new_rs["status"]["readyReplicas"] == 2;
            if old_zero && new_full && new_ready {
                break (old_rs, new_rs);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            let replicasets =
                list_owned_replicasets(&client, &ctx.base_url, "test-ns", "rollout-deployment")
                    .await?;
            let ships =
                get_json(&client, &ctx.base_url, "/api/v1/namespaces/test-ns/ships").await?;
            return Err(format!(
                "timed out waiting for deployment rolling update; replicasets={replicasets:?}; ships={ships:?}"
            )
            .into());
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    };

    assert_eq!(old_rs["spec"]["replicas"], 0);
    assert_eq!(new_rs["spec"]["replicas"], 2);
    assert_eq!(
        new_rs["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/demo:v2"
    );

    Ok(())
}

#[tokio::test]
async fn deployment_controller_deletes_managed_replicasets_on_deletion() -> Result<(), DynError> {
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
        "/apis/apps/v1/namespaces/test-ns/deployments",
        &deployment_manifest(
            "test-ns",
            "delete-deployment",
            "demo-delete",
            2,
            "example.com/images/demo:v1",
        ),
    )
    .await?;

    let replicaset = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "delete-deployment",
        Duration::from_secs(20),
        |rs| rs["spec"]["replicas"] == 2,
    )
    .await?;
    let replicaset_name = string_field(&replicaset, &["metadata", "name"])?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &replicaset_name,
        2,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/delete-deployment",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    wait_for_owned_replicaset_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "delete-deployment",
        0,
        Duration::from_secs(20),
    )
    .await?;
    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &replicaset_name,
        0,
        Duration::from_secs(20),
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

async fn wait_for_owned_replicaset_count(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    deployment_name: &str,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<Value>, DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replicasets =
            list_owned_replicasets(client, base_url, namespace, deployment_name).await?;
        if replicasets.len() == expected {
            return Ok(replicasets);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {expected} replicasets owned by {namespace}/{deployment_name}: {replicasets:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_owned_replicaset<F>(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    deployment_name: &str,
    timeout: Duration,
    predicate: F,
) -> Result<Value, DynError>
where
    F: Fn(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replicasets =
            list_owned_replicasets(client, base_url, namespace, deployment_name).await?;
        if let Some(replicaset) = replicasets.into_iter().find(|rs| predicate(rs)) {
            return Ok(replicaset);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for matching replicaset owned by {namespace}/{deployment_name}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_replicaset_ready_replicas(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    replicaset_name: &str,
    expected: i64,
    timeout: Duration,
) -> Result<Value, DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replicaset = get_json(
            client,
            base_url,
            &format!("/apis/apps/v1/namespaces/{namespace}/replicasets/{replicaset_name}"),
        )
        .await?;
        if replicaset["status"]["readyReplicas"].as_i64() == Some(expected) {
            return Ok(replicaset);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for replicaset {namespace}/{replicaset_name} to report {expected} ready replicas: {replicaset:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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

async fn list_owned_replicasets(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    deployment_name: &str,
) -> Result<Vec<Value>, DynError> {
    let list = get_json(
        client,
        base_url,
        &format!("/apis/apps/v1/namespaces/{namespace}/replicasets"),
    )
    .await?;
    let items = list["items"]
        .as_array()
        .ok_or("replicaset list response is missing items")?;

    Ok(items
        .iter()
        .filter(|replicaset| {
            replicaset["metadata"]["ownerReferences"]
                .as_array()
                .is_some_and(|owners| {
                    owners.iter().any(|owner| {
                        owner["kind"] == "Deployment"
                            && owner["name"] == deployment_name
                            && owner["controller"] == true
                    })
                })
        })
        .cloned()
        .collect())
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

async fn mark_labeled_ships_running(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    label_key: &str,
    label_value: &str,
) -> Result<(), DynError> {
    let list = get_json(
        client,
        base_url,
        &format!("/api/v1/namespaces/{namespace}/ships"),
    )
    .await?;
    let items = list["items"]
        .as_array()
        .ok_or("ship list response is missing items")?;

    for ship in items.iter().filter(|ship| {
        ship["metadata"]["labels"][label_key]
            .as_str()
            .is_some_and(|value| value == label_value)
    }) {
        let Some(name) = ship["metadata"]["name"].as_str() else {
            continue;
        };
        if ship["status"]["conditions"]
            .as_array()
            .is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|condition| condition["status"] == "Running")
            })
        {
            continue;
        }

        request_json(
            client,
            Method::PATCH,
            &format!("{base_url}/api/v1/namespaces/{namespace}/ships/{name}/status"),
            StatusCode::OK,
            Some(json!({
                "status": {
                    "conditions": [
                        {
                            "status": "Running",
                            "message": "integration test ready",
                            "timestamp": "2023-11-14T22:13:20Z"
                        }
                    ]
                }
            })),
        )
        .await?;
    }

    Ok(())
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

fn string_field<'a>(value: &'a Value, path: &[&str]) -> Result<String, DynError> {
    let mut current = value;
    for segment in path {
        current = &current[*segment];
    }
    current
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("missing string field at {}", path.join(".")).into())
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

fn deployment_manifest(
    namespace: &str,
    name: &str,
    app_label: &str,
    replicas: i32,
    image: &str,
) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "replicas": replicas,
            "selector": {
                "app": app_label
            },
            "shipTemplate": {
                "metadata": {
                    "labels": {
                        "app": app_label
                    }
                },
                "spec": {
                    "image": image,
                    "shipClass": "standard"
                }
            },
            "strategy": {
                "type": "RollingUpdate",
                "rollingUpdate": {
                    "maxSurge": 1,
                    "maxUnavailable": 0
                }
            },
            "revisionHistoryLimit": 2
        }
    })
}
