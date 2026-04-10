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
async fn ship_lifecycle_covers_scheduling_status_and_deletion() -> Result<(), DynError> {
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
        "/api/v1/storageclasses",
        &storageclass_manifest("standard"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/persistentvolumes",
        &persistent_volume_manifest("pv-lifecycle", "standard"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("stateful", 1, "1Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-a", 4, 8 * 1024 * 1024 * 1024, false),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims",
        &persistent_volume_claim_manifest("test-ns", "data-disk", "standard"),
    )
    .await?;
    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/data-disk",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "volumeName": "pv-lifecycle"
            }
        })),
    )
    .await?;
    request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/data-disk/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {
                "name": "data-disk",
                "namespace": "test-ns"
            },
            "status": {
                "phase": "Bound",
                "capacityBytes": 1_073_741_824_i64,
                "resizePending": false,
                "conditions": [
                    {
                        "status": "True",
                        "message": "bound",
                        "timestamp": "2023-11-14T22:25:00Z"
                    }
                ]
            }
        })),
    )
    .await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &stateful_ship_manifest(
            "test-ns",
            "stateful-ship",
            "stateful",
            "example.com/images/demo:v1",
            "data-disk",
        ),
    )
    .await?;

    let scheduled = wait_for_ship(
        &client,
        &ctx.base_url,
        "test-ns",
        "stateful-ship",
        Duration::from_secs(20),
        |ship| ship["spec"]["nodeName"] == "node-a",
    )
    .await?;
    assert_eq!(
        scheduled["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"],
        "data-disk"
    );

    let running = mark_ship_running(&client, &ctx.base_url, "test-ns", "stateful-ship").await?;
    assert_eq!(running["status"]["conditions"][0]["status"], "Running");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships/stateful-ship",
    )
    .await?;
    assert_eq!(fetched["status"]["conditions"][0]["status"], "Running");
    assert_eq!(fetched["status"]["ips"][0]["ipv4"], "10.0.0.10");

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/stateful-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    wait_for_missing(
        &client,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/stateful-ship",
            ctx.base_url
        ),
        Duration::from_secs(10),
    )
    .await?;

    let pvc = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims/data-disk",
    )
    .await?;
    assert_eq!(pvc["spec"]["volumeName"], "pv-lifecycle");

    Ok(())
}

#[tokio::test]
async fn deployment_lifecycle_covers_rollout_and_cleanup() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("standard", 1, "1Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-a", 4, 8 * 1024 * 1024 * 1024, false),
    )
    .await?;
    let _scheduler = ctx.start_scheduler()?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/deployments",
        &deployment_manifest(
            "test-ns",
            "demo-deployment",
            "demo-app",
            2,
            "example.com/images/demo:v1",
        ),
    )
    .await?;

    let initial_rs = wait_for_owned_deployment_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-deployment",
        Duration::from_secs(20),
        |rs| rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/demo:v1",
    )
    .await?;
    let initial_rs_name = string_field(&initial_rs, &["metadata", "name"])?;
    let initial_ships = wait_for_owned_scheduled_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    for ship in &initial_ships {
        assert_eq!(ship["spec"]["nodeName"], "node-a");
    }

    mark_labeled_ships_running(&client, &ctx.base_url, "test-ns", "app", "demo-app").await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/demo-deployment",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "shipTemplate": {
                    "metadata": {
                        "labels": {
                            "app": "demo-app"
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
        mark_labeled_ships_running(&client, &ctx.base_url, "test-ns", "app", "demo-app").await?;

        let replicasets =
            list_owned_deployment_replicasets(&client, &ctx.base_url, "test-ns", "demo-deployment")
                .await?;
        let old_rs = replicasets
            .iter()
            .find(|rs| rs["metadata"]["name"] == initial_rs_name)
            .cloned();
        let new_rs = replicasets
            .iter()
            .find(|rs| {
                rs["metadata"]["name"] != initial_rs_name
                    && rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/demo:v2"
            })
            .cloned();

        if let (Some(old_rs), Some(new_rs)) = (old_rs, new_rs) {
            if old_rs["spec"]["replicas"] == 0
                && new_rs["spec"]["replicas"] == 2
                && new_rs["status"]["readyReplicas"] == 2
            {
                break (old_rs, new_rs);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            let replicasets = list_owned_deployment_replicasets(
                &client,
                &ctx.base_url,
                "test-ns",
                "demo-deployment",
            )
            .await?;
            let ships =
                get_json(&client, &ctx.base_url, "/api/v1/namespaces/test-ns/ships").await?;
            return Err(format!(
                "timed out waiting for deployment lifecycle rollout; replicasets={replicasets:?}; ships={ships:?}"
            )
            .into());
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    };

    let new_rs_name = string_field(&new_rs, &["metadata", "name"])?;
    let new_ships = wait_for_owned_scheduled_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &new_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    assert_eq!(old_rs["spec"]["replicas"], 0);
    for ship in &new_ships {
        assert_eq!(ship["spec"]["nodeName"], "node-a");
        assert_eq!(ship["spec"]["image"], "example.com/images/demo:v2");
    }

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/demo-deployment",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    wait_for_owned_deployment_replicaset_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-deployment",
        0,
        Duration::from_secs(20),
    )
    .await?;
    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &new_rs_name,
        0,
        Duration::from_secs(20),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn fleet_lifecycle_covers_managed_ships_and_cleanup() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "tugboat-system").await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_cluster_network_class(&client, &ctx.base_url, "tenant-net").await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("standard", 1, "1Gi"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-a", 4, 8 * 1024 * 1024 * 1024, false),
    )
    .await?;
    let _scheduler = ctx.start_scheduler()?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets",
        &fleet_manifest(
            "test-ns",
            "demo-fleet",
            "tenant-net",
            &[
                ("frontend", 2, "example.com/images/frontend:v1"),
                ("api", 1, "example.com/images/api:v1"),
            ],
        ),
    )
    .await?;

    let frontend_rs = wait_for_owned_fleet_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-fleet",
        "frontend",
        Duration::from_secs(20),
        |_| true,
    )
    .await?;
    let api_rs = wait_for_owned_fleet_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-fleet",
        "api",
        Duration::from_secs(20),
        |_| true,
    )
    .await?;

    let frontend_rs_name = string_field(&frontend_rs, &["metadata", "name"])?;
    let api_rs_name = string_field(&api_rs, &["metadata", "name"])?;
    let frontend_ships = wait_for_owned_scheduled_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &frontend_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    let api_ships = wait_for_owned_scheduled_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &api_rs_name,
        1,
        Duration::from_secs(20),
    )
    .await?;

    for ship in frontend_ships.iter().chain(api_ships.iter()) {
        assert_eq!(ship["spec"]["nodeName"], "node-a");
        assert!(
            ship["spec"]["networkClassRef"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
    }

    mark_fleet_ships_running(&client, &ctx.base_url, "test-ns", "demo-fleet").await?;
    let fleet = wait_for_fleet_status(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-fleet",
        2,
        2,
        Duration::from_secs(30),
    )
    .await?;
    assert_eq!(fleet["status"]["readyComponents"], 2);

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/fleets/demo-fleet",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    wait_for_owned_fleet_replicaset_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-fleet",
        0,
        Duration::from_secs(20),
    )
    .await?;
    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &frontend_rs_name,
        0,
        Duration::from_secs(20),
    )
    .await?;
    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &api_rs_name,
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

async fn wait_for_ship<F>(
    client: &SecureClient,
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

async fn wait_for_missing(
    client: &SecureClient,
    url: &str,
    timeout: Duration,
) -> Result<(), DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let response = client.get(url).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let body = response.text().await?;
            return Err(format!("timed out waiting for deletion at {url}: {body}").into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_owned_deployment_replicaset_count(
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
            list_owned_deployment_replicasets(client, base_url, namespace, deployment_name).await?;
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

async fn wait_for_owned_deployment_replicaset<F>(
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
            list_owned_deployment_replicasets(client, base_url, namespace, deployment_name).await?;
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

async fn wait_for_owned_fleet_replicaset_count(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    fleet_name: &str,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<Value>, DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replicasets =
            list_owned_fleet_replicasets(client, base_url, namespace, fleet_name).await?;
        if replicasets.len() == expected {
            return Ok(replicasets);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {expected} replicasets owned by {namespace}/{fleet_name}: {replicasets:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_owned_fleet_replicaset<F>(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    fleet_name: &str,
    component_name: &str,
    timeout: Duration,
    predicate: F,
) -> Result<Value, DynError>
where
    F: Fn(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replicasets =
            list_owned_fleet_replicasets(client, base_url, namespace, fleet_name).await?;
        if let Some(replicaset) = replicasets.into_iter().find(|rs| {
            rs["metadata"]["labels"]["fleet-component"] == component_name && predicate(rs)
        }) {
            return Ok(replicaset);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for matching replicaset owned by {namespace}/{fleet_name} for component {component_name}"
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

async fn wait_for_owned_scheduled_ship_count(
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
        if ships.len() == expected
            && ships
                .iter()
                .all(|ship| ship["spec"]["nodeName"].as_str().is_some())
        {
            return Ok(ships);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {expected} scheduled ships owned by {namespace}/{owner_name}: {ships:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_fleet_status(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
    ready_components: i64,
    total_components: i64,
    timeout: Duration,
) -> Result<Value, DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let fleet = get_json(
            client,
            base_url,
            &format!("/apis/apps/v1/namespaces/{namespace}/fleets/{name}"),
        )
        .await?;
        if fleet["status"]["readyComponents"].as_i64() == Some(ready_components)
            && fleet["status"]["totalComponents"].as_i64() == Some(total_components)
        {
            return Ok(fleet);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for fleet {namespace}/{name} status ready={ready_components} total={total_components}: {fleet:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn list_owned_deployment_replicasets(
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

async fn list_owned_fleet_replicasets(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    fleet_name: &str,
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
                        owner["kind"] == "Fleet"
                            && owner["name"] == fleet_name
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

async fn mark_ship_running(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
) -> Result<Value, DynError> {
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
                ],
                "ips": [
                    {
                        "nic": "eth0",
                        "ipv4": "10.0.0.10"
                    }
                ]
            }
        })),
    )
    .await
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
        mark_ship_running(client, base_url, namespace, name).await?;
    }

    Ok(())
}

async fn mark_fleet_ships_running(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    fleet_name: &str,
) -> Result<(), DynError> {
    mark_labeled_ships_running(client, base_url, namespace, "fleet-name", fleet_name).await
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

async fn create_cluster_network_class(
    client: &SecureClient,
    base_url: &str,
    name: &str,
) -> Result<Value, DynError> {
    create_resource(
        client,
        base_url,
        "/api/v1/clusternetworkclasses",
        &cluster_network_class_manifest(name),
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

fn string_field(value: &Value, path: &[&str]) -> Result<String, DynError> {
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

fn storageclass_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "StorageClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "provisioner": "csi.tugboat.cloud/standard",
            "parameters": {
                "pool": "standard"
            },
            "reclaimPolicy": "Delete",
            "allowVolumeExpansion": true,
            "mountOptions": ["discard"]
        }
    })
}

fn persistent_volume_manifest(name: &str, storage_class_name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolume",
        "metadata": {
            "name": name
        },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "persistentVolumeReclaimPolicy": "Delete",
            "storageClassName": storage_class_name,
            "volumeMode": "Filesystem",
            "capacityBytes": 1_073_741_824_i64,
            "csi": {
                "driver": "csi.tugboat.cloud/standard",
                "volumeHandle": format!("handle-{name}"),
                "readOnly": false,
                "volumeAttributes": {
                    "pool": "standard"
                },
                "mountOptions": ["discard"]
            }
        }
    })
}

fn persistent_volume_claim_manifest(
    namespace: &str,
    name: &str,
    storage_class_name: &str,
) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "storageClassName": storage_class_name,
            "volumeMode": "Filesystem",
            "requestedCapacityBytes": 1_073_741_824_i64
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

fn stateful_ship_manifest(
    namespace: &str,
    name: &str,
    ship_class: &str,
    image: &str,
    claim_name: &str,
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
            "tolerations": [],
            "volumes": [
                {
                    "name": "data",
                    "persistentVolumeClaim": {
                        "claimName": claim_name
                    }
                }
            ],
            "volumeClaimRef": [
                {
                    "name": claim_name
                }
            ]
        }
    })
}

fn node_manifest(name: &str, cpu: u64, memory: u64, unschedulable: bool) -> Value {
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

fn cluster_network_class_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ClusterNetworkClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "subnet": "10.100.0.0/24",
            "cniPlugin": "bridge",
            "internetAccess": true,
            "clusterNetwork": false
        }
    })
}

fn fleet_manifest(
    namespace: &str,
    name: &str,
    network_class_name: &str,
    components: &[(&str, i32, &str)],
) -> Value {
    let components = components
        .iter()
        .map(|(component_name, replicas, image)| {
            json!({
                "name": component_name,
                "replicas": replicas,
                "shipTemplate": {
                    "metadata": {
                        "labels": {
                            "component": component_name
                        }
                    },
                    "spec": {
                        "image": image,
                        "shipClass": "standard"
                    }
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "apiVersion": "apps/v1",
        "kind": "Fleet",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "networkClassName": network_class_name,
            "components": components
        }
    })
}
