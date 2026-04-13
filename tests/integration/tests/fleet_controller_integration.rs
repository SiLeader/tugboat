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
async fn fleet_controller_creates_replicasets_and_ships_for_components() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_cluster_network_class(&client, &ctx.base_url, "tenant-net").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    let fleet = create_resource(
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
    let fleet_uid = string_field(&fleet, &["metadata", "uid"])?;

    let frontend_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-fleet",
        "frontend",
        Duration::from_secs(20),
        |rs| rs["spec"]["replicas"] == 2,
    )
    .await?;
    let api_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "demo-fleet",
        "api",
        Duration::from_secs(20),
        |rs| rs["spec"]["replicas"] == 1,
    )
    .await?;

    assert_owned_by_fleet(&frontend_rs, "demo-fleet", &fleet_uid);
    assert_owned_by_fleet(&api_rs, "demo-fleet", &fleet_uid);

    let frontend_rs_name = string_field(&frontend_rs, &["metadata", "name"])?;
    let frontend_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &frontend_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    for ship in frontend_ships {
        assert_eq!(ship["spec"]["image"], "example.com/images/frontend:v1");
        assert_eq!(ship["metadata"]["labels"]["component"], "frontend");
        assert_eq!(ship["metadata"]["labels"]["fleet-name"], "demo-fleet");
        assert_eq!(ship["metadata"]["labels"]["fleet-component"], "frontend");
    }

    let api_rs_name = string_field(&api_rs, &["metadata", "name"])?;
    let api_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &api_rs_name,
        1,
        Duration::from_secs(20),
    )
    .await?;
    assert_eq!(api_ships[0]["spec"]["image"], "example.com/images/api:v1");
    assert_eq!(api_ships[0]["metadata"]["labels"]["component"], "api");
    assert_eq!(
        api_ships[0]["metadata"]["labels"]["fleet-name"],
        "demo-fleet"
    );
    assert_eq!(api_ships[0]["metadata"]["labels"]["fleet-component"], "api");

    Ok(())
}

#[tokio::test]
async fn fleet_controller_injects_shared_network_and_reports_status() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_cluster_network_class(&client, &ctx.base_url, "tenant-net").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets",
        &fleet_manifest(
            "test-ns",
            "network-fleet",
            "tenant-net",
            &[
                ("frontend", 2, "example.com/images/frontend:v1"),
                ("api", 1, "example.com/images/api:v1"),
            ],
        ),
    )
    .await?;

    let frontend_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "network-fleet",
        "frontend",
        Duration::from_secs(20),
        |_| true,
    )
    .await?;
    let frontend_rs_name = string_field(&frontend_rs, &["metadata", "name"])?;

    assert_has_network_class(
        &frontend_rs["spec"]["shipTemplate"]["spec"]["networkClassRef"],
        "tenant-net",
    );

    let frontend_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &frontend_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    for ship in &frontend_ships {
        assert_has_network_class(&ship["spec"]["networkClassRef"], "tenant-net");
    }

    mark_fleet_ships_running(&client, &ctx.base_url, "test-ns", "network-fleet").await?;
    let fleet = wait_for_fleet_status(
        &client,
        &ctx.base_url,
        "test-ns",
        "network-fleet",
        2,
        2,
        Duration::from_secs(30),
    )
    .await?;
    assert_eq!(fleet["status"]["readyComponents"], 2);
    assert_eq!(fleet["status"]["totalComponents"], 2);

    Ok(())
}

#[tokio::test]
async fn fleet_controller_rolls_component_update_to_new_replicaset() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_cluster_network_class(&client, &ctx.base_url, "tenant-net").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets",
        &fleet_manifest(
            "test-ns",
            "rollout-fleet",
            "tenant-net",
            &[
                ("frontend", 2, "example.com/images/frontend:v1"),
                ("api", 1, "example.com/images/api:v1"),
            ],
        ),
    )
    .await?;

    let initial_frontend_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "rollout-fleet",
        "frontend",
        Duration::from_secs(20),
        |rs| rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/frontend:v1",
    )
    .await?;
    let initial_frontend_rs_name = string_field(&initial_frontend_rs, &["metadata", "name"])?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_frontend_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    mark_fleet_ships_running(&client, &ctx.base_url, "test-ns", "rollout-fleet").await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/fleets/rollout-fleet",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "components": [
                    {
                        "name": "frontend",
                        "replicas": 2,
                        "shipTemplate": {
                            "metadata": {
                                "labels": {
                                    "component": "frontend"
                                }
                            },
                            "spec": {
                                "image": "example.com/images/frontend:v2",
                                "shipClass": "standard"
                            }
                        }
                    },
                    {
                        "name": "api",
                        "replicas": 1,
                        "shipTemplate": {
                            "metadata": {
                                "labels": {
                                    "component": "api"
                                }
                            },
                            "spec": {
                                "image": "example.com/images/api:v1",
                                "shipClass": "standard"
                            }
                        }
                    }
                ]
            }
        })),
    )
    .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let new_rs = loop {
        mark_fleet_ships_running(&client, &ctx.base_url, "test-ns", "rollout-fleet").await?;

        let replicasets =
            list_owned_replicasets(&client, &ctx.base_url, "test-ns", "rollout-fleet").await?;
        let new_rs = replicasets
            .iter()
            .find(|rs| {
                rs["metadata"]["labels"]["fleet-component"] == "frontend"
                    && rs["metadata"]["name"] != initial_frontend_rs_name
                    && rs["spec"]["shipTemplate"]["spec"]["image"]
                        == "example.com/images/frontend:v2"
            })
            .cloned();

        if let Some(new_rs) = new_rs
            && new_rs["spec"]["replicas"] == 2
        {
            break new_rs;
        }

        if tokio::time::Instant::now() >= deadline {
            let replicasets =
                list_owned_replicasets(&client, &ctx.base_url, "test-ns", "rollout-fleet").await?;
            return Err(format!(
                "timed out waiting for fleet rollout; replicasets={replicasets:?}"
            )
            .into());
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    };

    assert_eq!(new_rs["spec"]["replicas"], 2);
    assert_eq!(
        new_rs["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/frontend:v2"
    );
    let new_rs_name = string_field(&new_rs, &["metadata", "name"])?;
    let new_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &new_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    for ship in new_ships {
        assert_eq!(ship["spec"]["image"], "example.com/images/frontend:v2");
        assert_eq!(ship["metadata"]["labels"]["fleet-name"], "rollout-fleet");
        assert_eq!(ship["metadata"]["labels"]["fleet-component"], "frontend");
    }

    Ok(())
}

#[tokio::test]
async fn fleet_controller_preserves_migrating_ship_during_rollout() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_cluster_network_class(&client, &ctx.base_url, "tenant-net").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets",
        &fleet_manifest(
            "test-ns",
            "migration-aware-fleet",
            "tenant-net",
            &[("frontend", 2, "example.com/images/frontend:v1")],
        ),
    )
    .await?;

    let initial_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "migration-aware-fleet",
        "frontend",
        Duration::from_secs(20),
        |rs| rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/frontend:v1",
    )
    .await?;
    let initial_rs_name = string_field(&initial_rs, &["metadata", "name"])?;

    let initial_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    mark_fleet_ships_running(&client, &ctx.base_url, "test-ns", "migration-aware-fleet").await?;

    let migrating_ship_name = string_field(&initial_ships[0], &["metadata", "name"])?;
    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/{}/status",
            ctx.base_url, migrating_ship_name
        ),
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
                "migration": {
                    "phase": "Migrating",
                    "message": "integration test migration",
                    "timestamp": "2023-11-14T22:13:20Z"
                }
            }
        })),
    )
    .await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/fleets/migration-aware-fleet",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "components": [
                    {
                        "name": "frontend",
                        "replicas": 2,
                        "shipTemplate": {
                            "metadata": {
                                "labels": {
                                    "component": "frontend"
                                }
                            },
                            "spec": {
                                "image": "example.com/images/frontend:v2",
                                "shipClass": "standard"
                            }
                        }
                    }
                ]
            }
        })),
    )
    .await?;

    let new_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "migration-aware-fleet",
        "frontend",
        Duration::from_secs(30),
        |rs| {
            rs["metadata"]["name"] != initial_rs_name
                && rs["spec"]["shipTemplate"]["spec"]["image"] == "example.com/images/frontend:v2"
                && rs["spec"]["replicas"] == 2
        },
    )
    .await?;
    let new_rs_name = string_field(&new_rs, &["metadata", "name"])?;

    let new_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &new_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;
    assert_eq!(new_ships.len(), 2);
    mark_fleet_ships_running(&client, &ctx.base_url, "test-ns", "migration-aware-fleet").await?;

    let retained_old_ships = wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_rs_name,
        1,
        Duration::from_secs(30),
    )
    .await?;
    assert_eq!(
        retained_old_ships[0]["metadata"]["name"].as_str(),
        Some(migrating_ship_name.as_str())
    );
    assert_eq!(
        retained_old_ships[0]["status"]["migration"]["phase"].as_str(),
        Some("Migrating")
    );

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/{}/status",
            ctx.base_url, migrating_ship_name
        ),
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
                "migration": null
            }
        })),
    )
    .await?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &initial_rs_name,
        0,
        Duration::from_secs(30),
    )
    .await?;
    wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "migration-aware-fleet",
        "frontend",
        Duration::from_secs(30),
        |rs| rs["metadata"]["name"] == new_rs_name,
    )
    .await?;
    wait_for_owned_replicaset_count(
        &client,
        &ctx.base_url,
        "test-ns",
        "migration-aware-fleet",
        1,
        Duration::from_secs(30),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn fleet_controller_deletes_managed_replicasets_and_ships_on_deletion() -> Result<(), DynError>
{
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_cluster_network_class(&client, &ctx.base_url, "tenant-net").await?;
    let _controller_manager = ctx.start_controller_manager().await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets",
        &fleet_manifest(
            "test-ns",
            "delete-fleet",
            "tenant-net",
            &[
                ("frontend", 2, "example.com/images/frontend:v1"),
                ("api", 1, "example.com/images/api:v1"),
            ],
        ),
    )
    .await?;

    let frontend_rs = wait_for_owned_replicaset(
        &client,
        &ctx.base_url,
        "test-ns",
        "delete-fleet",
        "frontend",
        Duration::from_secs(20),
        |_| true,
    )
    .await?;
    let frontend_rs_name = string_field(&frontend_rs, &["metadata", "name"])?;

    wait_for_owned_ship_count(
        &client,
        &ctx.base_url,
        "test-ns",
        &frontend_rs_name,
        2,
        Duration::from_secs(20),
    )
    .await?;

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/fleets/delete-fleet",
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
        "delete-fleet",
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

fn test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

async fn wait_for_owned_replicaset_count(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    fleet_name: &str,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<Value>, DynError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replicasets = list_owned_replicasets(client, base_url, namespace, fleet_name).await?;
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

async fn wait_for_owned_replicaset<F>(
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
        let replicasets = list_owned_replicasets(client, base_url, namespace, fleet_name).await?;
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

async fn list_owned_replicasets(
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

async fn mark_fleet_ships_running(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    fleet_name: &str,
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

    for ship in items
        .iter()
        .filter(|ship| ship["metadata"]["labels"]["fleet-name"] == fleet_name)
    {
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

fn assert_owned_by_fleet(replicaset: &Value, fleet_name: &str, fleet_uid: &str) {
    assert!(
        replicaset["metadata"]["ownerReferences"]
            .as_array()
            .is_some_and(|owners| owners.iter().any(|owner| {
                owner["kind"] == "Fleet"
                    && owner["name"] == fleet_name
                    && owner["uid"] == fleet_uid
                    && owner["controller"] == true
            }))
    );
}

fn assert_has_network_class(network_refs: &Value, expected_name: &str) {
    assert!(
        network_refs
            .as_array()
            .is_some_and(|refs| refs.iter().any(|network| {
                network["kind"] == "ClusterNetworkClass"
                    && network["apiGroup"] == "core"
                    && network["name"] == expected_name
            })),
        "expected networkClassRef to contain {expected_name}: {network_refs:?}"
    );
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
