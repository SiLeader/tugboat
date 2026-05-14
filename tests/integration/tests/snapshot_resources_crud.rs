#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn snapshot_resources_support_crud_round_trip() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "snapshot-ns").await?;

    let class = create_resource(
        &client,
        &ctx.base_url,
        "/apis/snapshot/v1/volumesnapshotclasses",
        &volume_snapshot_class_manifest("fast-snapshots"),
    )
    .await?;
    assert_eq!(class["kind"], "VolumeSnapshotClass");
    assert_eq!(class["metadata"]["name"], "fast-snapshots");
    assert_eq!(class["spec"]["deletionPolicy"], "Delete");

    let snapshot = create_resource(
        &client,
        &ctx.base_url,
        "/apis/snapshot/v1/namespaces/snapshot-ns/volumesnapshots",
        &volume_snapshot_manifest("snapshot-ns", "daily-snapshot", "data-pvc"),
    )
    .await?;
    assert_eq!(snapshot["kind"], "VolumeSnapshot");
    assert_eq!(
        snapshot["spec"]["source"]["persistentVolumeClaimName"],
        "data-pvc"
    );

    let content = create_resource(
        &client,
        &ctx.base_url,
        "/apis/snapshot/v1/volumesnapshotcontents",
        &volume_snapshot_content_manifest("daily-snapshot-content", "daily-snapshot"),
    )
    .await?;
    assert_eq!(content["kind"], "VolumeSnapshotContent");
    assert_eq!(content["spec"]["source"]["volumeHandle"], "pv-handle-1");

    let patched_class = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/snapshot/v1/volumesnapshotclasses/fast-snapshots",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "parameters": {
                    "pool": "ssd"
                }
            }
        })),
    )
    .await?;
    assert_eq!(patched_class["spec"]["parameters"]["pool"], "ssd");

    let status_patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/snapshot/v1/namespaces/snapshot-ns/volumesnapshots/daily-snapshot/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "status": {
                "boundVolumeSnapshotContentName": "daily-snapshot-content",
                "readyToUse": true,
                "restoreSizeBytes": 1024
            }
        })),
    )
    .await?;
    assert_eq!(
        status_patched["status"]["boundVolumeSnapshotContentName"],
        "daily-snapshot-content"
    );
    assert_eq!(status_patched["status"]["readyToUse"], true);

    let fetched_snapshot = get_json(
        &client,
        &ctx.base_url,
        "/apis/snapshot/v1/namespaces/snapshot-ns/volumesnapshots/daily-snapshot",
    )
    .await?;
    assert_eq!(fetched_snapshot["status"]["restoreSizeBytes"], 1024);

    for path in [
        "/apis/snapshot/v1/volumesnapshotcontents/daily-snapshot-content",
        "/apis/snapshot/v1/namespaces/snapshot-ns/volumesnapshots/daily-snapshot",
        "/apis/snapshot/v1/volumesnapshotclasses/fast-snapshots",
    ] {
        let deleted = request_json(
            &client,
            Method::DELETE,
            &format!("{}{}", ctx.base_url, path),
            StatusCode::OK,
            None,
        )
        .await?;
        assert!(deleted["metadata"]["name"].is_string());
    }

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
) -> Result<(), DynError> {
    create_resource(
        client,
        base_url,
        "/api/v1/namespaces",
        &json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": name
            }
        }),
    )
    .await?;
    Ok(())
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

fn volume_snapshot_class_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "snapshot/v1",
        "kind": "VolumeSnapshotClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "driver": "csi.tugboat.cloud/fast",
            "deletionPolicy": "Delete",
            "parameters": {
                "tier": "gold"
            },
            "snapshotterSecretRef": {
                "name": "snapshotter-secret",
                "namespace": "snapshot-ns"
            }
        }
    })
}

fn volume_snapshot_manifest(namespace: &str, name: &str, pvc_name: &str) -> Value {
    json!({
        "apiVersion": "snapshot/v1",
        "kind": "VolumeSnapshot",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "volumeSnapshotClassName": "fast-snapshots",
            "source": {
                "persistentVolumeClaimName": pvc_name
            }
        }
    })
}

fn volume_snapshot_content_manifest(name: &str, snapshot_name: &str) -> Value {
    json!({
        "apiVersion": "snapshot/v1",
        "kind": "VolumeSnapshotContent",
        "metadata": {
            "name": name
        },
        "spec": {
            "driver": "csi.tugboat.cloud/fast",
            "deletionPolicy": "Retain",
            "volumeSnapshotRef": {
                "name": snapshot_name,
                "namespace": "snapshot-ns",
                "uid": "snapshot-uid"
            },
            "source": {
                "volumeHandle": "pv-handle-1"
            },
            "volumeSnapshotClassName": "fast-snapshots",
            "sourceVolumeHandle": "pv-handle-1"
        }
    })
}
