use std::error::Error;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

const LIVE_MIGRATION_ENABLED_ENV: &str = "TUGBOAT_LIVE_MIGRATION_E2E";
const APISERVER_URL_ENV: &str = "TUGBOAT_TEST_APISERVER_URL";
const SOURCE_NODE_ENV: &str = "TUGBOAT_LIVE_MIGRATION_SOURCE_NODE";
const TARGET_NODE_ENV: &str = "TUGBOAT_LIVE_MIGRATION_TARGET_NODE";
const IMAGE_ENV: &str = "TUGBOAT_LIVE_MIGRATION_IMAGE";

#[tokio::test]
#[ignore = "requires external migration environment with agent/QEMU"]
async fn live_migration_happy_path_transitions_to_completed() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    let run_id = unique_suffix()?;
    let namespace = format!("migration-e2e-{run_id}");
    let ship_class = format!("migration-enabled-{run_id}");
    let storage_class = format!("migration-rwx-{run_id}");
    let pv = format!("migration-rwx-pv-{run_id}");
    let pvc = format!("migration-rwx-pvc-{run_id}");
    let ship = format!("migration-happy-{run_id}");

    create_namespace(&client, &ctx.base_url, &namespace).await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/storageclasses",
        &storage_class_manifest(&storage_class),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/persistentvolumes",
        &persistent_volume_manifest(&pv, &storage_class, "ReadWriteMany"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        &format!("/api/v1/namespaces/{namespace}/persistentvolumeclaims"),
        &persistent_volume_claim_manifest(&namespace, &pvc, &storage_class, "ReadWriteMany"),
    )
    .await?;
    bind_pvc(&client, &ctx.base_url, &namespace, &pvc, &pv).await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &migratable_shipclass_manifest(&ship_class),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        &format!("/api/v1/namespaces/{namespace}/ships"),
        &stateful_ship_manifest(
            &namespace,
            &ship,
            &ship_class,
            &ctx.image,
            &pvc,
            &ctx.source_node,
        ),
    )
    .await?;

    wait_for_ship(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        Duration::from_secs(180),
        |ship| {
            ship["spec"]["nodeName"] == ctx.source_node
                && ship["status"]["conditions"]
                    .as_array()
                    .is_some_and(|conditions| {
                        conditions
                            .iter()
                            .any(|condition| condition["status"] == "Running")
                    })
        },
    )
    .await?;

    patch_target_node(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        &ctx.target_node,
        StatusCode::OK,
    )
    .await?;

    let completed = wait_for_ship(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        Duration::from_secs(300),
        |ship| ship["status"]["migration"]["phase"] == "Completed",
    )
    .await?;

    let phases = collect_migration_phases(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        Duration::from_secs(5),
    )
    .await?;
    assert_phase_sequence(&phases, &["Pending", "Ready", "Migrating", "Completed"])?;

    assert_eq!(completed["spec"]["nodeName"], ctx.target_node);
    assert!(completed["spec"]["targetNodeName"].is_null());
    assert_eq!(
        completed["status"]["migration"]["targetNodeName"],
        ctx.target_node
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires external migration environment with agent/QEMU"]
async fn live_migration_rejects_rwo_storage_during_preflight() -> Result<(), DynError> {
    let _guard = test_lock().lock().await;
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    let run_id = unique_suffix()?;
    let namespace = format!("migration-e2e-{run_id}");
    let ship_class = format!("migration-enabled-{run_id}");
    let storage_class = format!("migration-rwo-{run_id}");
    let pv = format!("migration-rwo-pv-{run_id}");
    let pvc = format!("migration-rwo-pvc-{run_id}");
    let ship = format!("migration-rwo-{run_id}");

    create_namespace(&client, &ctx.base_url, &namespace).await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/storageclasses",
        &storage_class_manifest(&storage_class),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/persistentvolumes",
        &persistent_volume_manifest(&pv, &storage_class, "ReadWriteOnce"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        &format!("/api/v1/namespaces/{namespace}/persistentvolumeclaims"),
        &persistent_volume_claim_manifest(&namespace, &pvc, &storage_class, "ReadWriteOnce"),
    )
    .await?;
    bind_pvc(&client, &ctx.base_url, &namespace, &pvc, &pv).await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &migratable_shipclass_manifest(&ship_class),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        &format!("/api/v1/namespaces/{namespace}/ships"),
        &stateful_ship_manifest(
            &namespace,
            &ship,
            &ship_class,
            &ctx.image,
            &pvc,
            &ctx.source_node,
        ),
    )
    .await?;

    wait_for_ship(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        Duration::from_secs(180),
        |ship| {
            ship["spec"]["nodeName"] == ctx.source_node
                && ship["status"]["conditions"]
                    .as_array()
                    .is_some_and(|conditions| {
                        conditions
                            .iter()
                            .any(|condition| condition["status"] == "Running")
                    })
        },
    )
    .await?;

    patch_target_node(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        &ctx.target_node,
        StatusCode::OK,
    )
    .await?;

    let failed = wait_for_ship(
        &client,
        &ctx.base_url,
        &namespace,
        &ship,
        Duration::from_secs(120),
        |ship| ship["status"]["migration"]["phase"] == "Failed",
    )
    .await?;

    assert_eq!(failed["spec"]["nodeName"], ctx.source_node);
    assert_eq!(failed["spec"]["targetNodeName"], ctx.target_node);
    assert!(
        failed["status"]["migration"]["message"]
            .as_str()
            .is_some_and(|message| {
                message.contains("preflight failed") && message.contains("ReadWriteMany")
            }),
        "unexpected migration failure message: {}",
        failed["status"]["migration"]["message"]
    );

    Ok(())
}

struct ExternalMigrationContext {
    base_url: String,
    source_node: String,
    target_node: String,
    image: String,
}

async fn setup_or_skip() -> Result<Option<ExternalMigrationContext>, DynError> {
    if std::env::var_os(LIVE_MIGRATION_ENABLED_ENV).is_none() {
        eprintln!("skipping live migration e2e: set {LIVE_MIGRATION_ENABLED_ENV}=1 to enable");
        return Ok(None);
    }

    let Some(base_url) = std::env::var_os(APISERVER_URL_ENV) else {
        eprintln!("skipping live migration e2e: set {APISERVER_URL_ENV}");
        return Ok(None);
    };
    let Some(source_node) = std::env::var_os(SOURCE_NODE_ENV) else {
        eprintln!("skipping live migration e2e: set {SOURCE_NODE_ENV}");
        return Ok(None);
    };
    let Some(target_node) = std::env::var_os(TARGET_NODE_ENV) else {
        eprintln!("skipping live migration e2e: set {TARGET_NODE_ENV}");
        return Ok(None);
    };
    let Some(image) = std::env::var_os(IMAGE_ENV) else {
        eprintln!("skipping live migration e2e: set {IMAGE_ENV}");
        return Ok(None);
    };

    let base_url = base_url.to_string_lossy().into_owned();
    let source_node = source_node.to_string_lossy().into_owned();
    let target_node = target_node.to_string_lossy().into_owned();
    let image = image.to_string_lossy().into_owned();

    if source_node == target_node {
        return Err(format!(
            "{SOURCE_NODE_ENV} and {TARGET_NODE_ENV} must reference different nodes"
        )
        .into());
    }

    let client = Client::new();
    let source = get_json(&client, &base_url, &format!("/api/v1/nodes/{source_node}")).await?;
    let target = get_json(&client, &base_url, &format!("/api/v1/nodes/{target_node}")).await?;
    assert_eq!(source["metadata"]["name"], source_node);
    assert_eq!(target["metadata"]["name"], target_node);

    Ok(Some(ExternalMigrationContext {
        base_url,
        source_node,
        target_node,
        image,
    }))
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
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn collect_migration_phases(
    client: &Client,
    base_url: &str,
    namespace: &str,
    name: &str,
    settle_time: Duration,
) -> Result<Vec<String>, DynError> {
    let deadline = tokio::time::Instant::now() + settle_time;
    let mut phases = Vec::new();
    loop {
        let ship = get_json(
            client,
            base_url,
            &format!("/api/v1/namespaces/{namespace}/ships/{name}"),
        )
        .await?;
        if let Some(phase) = ship["status"]["migration"]["phase"]
            .as_str()
            .map(str::to_string)
            && phases.last() != Some(&phase)
        {
            phases.push(phase);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(phases);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn assert_phase_sequence(phases: &[String], expected: &[&str]) -> Result<(), DynError> {
    let mut index = 0;
    for phase in phases {
        if index < expected.len() && phase == expected[index] {
            index += 1;
        }
    }
    if index == expected.len() {
        Ok(())
    } else {
        Err(format!(
            "missing migration phase sequence {:?} in observed phases {:?}",
            expected, phases
        )
        .into())
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

async fn patch_target_node(
    client: &Client,
    base_url: &str,
    namespace: &str,
    name: &str,
    target_node_name: &str,
    expected_status: StatusCode,
) -> Result<Value, DynError> {
    request_json(
        client,
        Method::PATCH,
        &format!("{base_url}/api/v1/namespaces/{namespace}/ships/{name}"),
        expected_status,
        Some(json!({
            "spec": {
                "targetNodeName": target_node_name
            }
        })),
    )
    .await
}

async fn bind_pvc(
    client: &Client,
    base_url: &str,
    namespace: &str,
    pvc_name: &str,
    pv_name: &str,
) -> Result<(), DynError> {
    request_json(
        client,
        Method::PATCH,
        &format!("{base_url}/api/v1/namespaces/{namespace}/persistentvolumeclaims/{pvc_name}"),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "volumeName": pv_name
            }
        })),
    )
    .await?;
    request_json(
        client,
        Method::PUT,
        &format!(
            "{base_url}/api/v1/namespaces/{namespace}/persistentvolumeclaims/{pvc_name}/status"
        ),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {
                "name": pvc_name,
                "namespace": namespace
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
    Ok(())
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

fn unique_suffix() -> Result<String, DynError> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(format!("{}-{}", std::process::id(), now.as_millis()))
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

fn storage_class_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "StorageClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "provisioner": "csi.tugboat.dev/standard",
            "parameters": {
                "pool": "standard"
            },
            "reclaimPolicy": "Delete",
            "allowVolumeExpansion": true
        }
    })
}

fn persistent_volume_manifest(name: &str, storage_class_name: &str, access_mode: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolume",
        "metadata": {
            "name": name
        },
        "spec": {
            "accessModes": [access_mode],
            "persistentVolumeReclaimPolicy": "Delete",
            "storageClassName": storage_class_name,
            "volumeMode": "Filesystem",
            "capacityBytes": 1_073_741_824_i64,
            "csi": {
                "driver": "csi.tugboat.dev/standard",
                "volumeHandle": format!("handle-{name}"),
                "readOnly": false,
                "volumeAttributes": {
                    "pool": "standard"
                }
            }
        }
    })
}

fn persistent_volume_claim_manifest(
    namespace: &str,
    name: &str,
    storage_class_name: &str,
    access_mode: &str,
) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "accessModes": [access_mode],
            "storageClassName": storage_class_name,
            "volumeMode": "Filesystem",
            "requestedCapacityBytes": 1_073_741_824_i64
        }
    })
}

fn migratable_shipclass_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ShipClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "cpu": {
                "architecture": "x64",
                "cores": 1
            },
            "memory": {
                "size": "1Gi"
            },
            "migration": {}
        }
    })
}

fn stateful_ship_manifest(
    namespace: &str,
    name: &str,
    ship_class: &str,
    image: &str,
    claim_name: &str,
    node_name: &str,
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
