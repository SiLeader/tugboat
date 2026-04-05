#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn persistent_volume_supports_status_updates() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/storageclasses",
        &storageclass_manifest("standard"),
    )
    .await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/persistentvolumes",
        &persistent_volume_manifest("pv-1", "standard"),
    )
    .await?;
    assert_eq!(created["kind"], "PersistentVolume");
    assert_eq!(created["metadata"]["name"], "pv-1");
    assert_eq!(created["spec"]["storageClassName"], "standard");

    let fetched = get_json(&client, &ctx.base_url, "/api/v1/persistentvolumes/pv-1").await?;
    assert_eq!(fetched["spec"]["capacityBytes"], 1_073_741_824_i64);
    assert_eq!(fetched["spec"]["csi"]["driver"], "csi.tugboat.dev/standard");

    let updated = request_json(
        &client,
        Method::PUT,
        &format!("{}/api/v1/persistentvolumes/pv-1/status", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "PersistentVolume",
            "metadata": {
                "name": "pv-1"
            },
            "status": {
                "phase": "Available",
                "capacityBytes": 1_073_741_824_i64,
                "nodeExpansionRequired": false,
                "conditions": [
                    {
                        "status": "True",
                        "message": "available",
                        "timestamp": "2023-11-14T22:20:00Z"
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(updated["status"]["phase"], "Available");
    assert_eq!(updated["status"]["conditions"][0]["message"], "available");

    let refetched = get_json(&client, &ctx.base_url, "/api/v1/persistentvolumes/pv-1").await?;
    assert_eq!(refetched["status"]["phase"], "Available");
    assert_eq!(refetched["status"]["capacityBytes"], 1_073_741_824_i64);

    Ok(())
}

#[tokio::test]
async fn persistent_volume_claim_supports_binding_status_updates() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/storageclasses",
        &storageclass_manifest("standard"),
    )
    .await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims",
        &persistent_volume_claim_manifest("test-ns", "pvc-1", "standard"),
    )
    .await?;
    assert_eq!(created["kind"], "PersistentVolumeClaim");
    assert_eq!(created["metadata"]["name"], "pvc-1");
    assert_eq!(created["spec"]["storageClassName"], "standard");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims/pvc-1",
    )
    .await?;
    assert_eq!(fetched["spec"]["requestedCapacityBytes"], 1_073_741_824_i64);
    assert_eq!(fetched["spec"]["accessModes"][0], "ReadWriteOnce");

    let updated = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/pvc-1/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {
                "name": "pvc-1",
                "namespace": "test-ns"
            },
            "status": {
                "phase": "Bound",
                "capacityBytes": 1_073_741_824_i64,
                "resizePending": false,
                "conditions": [
                    {
                        "status": "True",
                        "message": "bound-to-pv-1",
                        "timestamp": "2023-11-14T22:25:00Z"
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(updated["status"]["phase"], "Bound");
    assert_eq!(
        updated["status"]["conditions"][0]["message"],
        "bound-to-pv-1"
    );

    let spec_updated = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/pvc-1",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "volumeName": "pv-1"
            }
        })),
    )
    .await?;
    assert_eq!(spec_updated["spec"]["volumeName"], "pv-1");

    let refetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims/pvc-1",
    )
    .await?;
    assert_eq!(refetched["spec"]["volumeName"], "pv-1");
    assert_eq!(refetched["status"]["phase"], "Bound");

    Ok(())
}

#[tokio::test]
async fn persistent_volume_can_be_deleted() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
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
        &persistent_volume_manifest("pv-delete", "standard"),
    )
    .await?;

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!("{}/api/v1/persistentvolumes/pv-delete", ctx.base_url),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "pv-delete");

    let response = client
        .get(format!(
            "{}/api/v1/persistentvolumes/pv-delete",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn persistent_volume_claim_can_be_deleted() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = Client::new();
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/storageclasses",
        &storageclass_manifest("standard"),
    )
    .await?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims",
        &persistent_volume_claim_manifest("test-ns", "pvc-delete", "standard"),
    )
    .await?;

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/pvc-delete",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "pvc-delete");

    let response = client
        .get(format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/pvc-delete",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

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

fn storageclass_manifest(name: &str) -> Value {
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
                "driver": "csi.tugboat.dev/standard",
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
