#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn shipclass_can_be_created_read_and_listed() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    let small = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("small", 2, "4Gi"),
    )
    .await?;
    assert_eq!(small["kind"], "ShipClass");
    assert_eq!(small["metadata"]["name"], "small");
    assert_eq!(small["spec"]["cpu"]["cores"], 2);
    assert_eq!(small["spec"]["memory"]["size"], "4Gi");

    let fetched = get_json(&client, &ctx.base_url, "/api/v1/shipclasses/small").await?;
    assert_eq!(fetched["metadata"]["name"], "small");
    assert_eq!(fetched["spec"]["cpu"]["architecture"], "x64");

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/shipclasses",
        &shipclass_manifest("large", 8, "16Gi"),
    )
    .await?;

    let listed = get_json(&client, &ctx.base_url, "/api/v1/shipclasses").await?;
    assert_eq!(listed["kind"], "List");
    let items = listed["items"]
        .as_array()
        .ok_or("shipclass list response is missing items")?;

    for name in ["small", "large"] {
        assert!(
            items.iter().any(|item| item["metadata"]["name"] == name),
            "shipclass list did not include {name}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn node_supports_crud_and_status_updates() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/nodes",
        &node_manifest("node-1"),
    )
    .await?;

    let fetched = get_json(&client, &ctx.base_url, "/api/v1/nodes/node-1").await?;
    assert_eq!(fetched["metadata"]["name"], "node-1");
    assert_eq!(fetched["spec"]["resource"]["cpu"], 4);
    assert_eq!(
        fetched["metadata"]["labels"]["topology.kubernetes.io/zone"],
        "zone-a"
    );

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!("{}/api/v1/nodes/node-1", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "labels": {
                    "node-role.kubernetes.io/worker": "true"
                }
            }
        })),
    )
    .await?;
    assert_eq!(
        patched["metadata"]["labels"]["node-role.kubernetes.io/worker"],
        "true"
    );

    let status_patched = request_json(
        &client,
        Method::PATCH,
        &format!("{}/api/v1/nodes/node-1/status", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "status": {
                "conditions": [
                    {
                        "type": "Ready",
                        "status": "True",
                        "message": "kubelet-ready"
                    }
                ],
                "cniPlugins": [
                    {
                        "name": "bridge",
                        "ready": true,
                        "message": "initialized"
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(status_patched["status"]["conditions"][0]["type"], "Ready");
    assert_eq!(status_patched["status"]["cniPlugins"][0]["name"], "bridge");

    let status_replaced = request_json(
        &client,
        Method::PUT,
        &format!("{}/api/v1/nodes/node-1/status", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "Node",
            "metadata": {
                "name": "node-1"
            },
            "status": {
                "conditions": [
                    {
                        "type": "Ready",
                        "status": "False",
                        "message": "maintenance"
                    }
                ],
                "cniPlugins": [
                    {
                        "name": "bridge",
                        "ready": false,
                        "message": "draining"
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(
        status_replaced["status"]["conditions"][0]["status"],
        "False"
    );
    assert_eq!(status_replaced["status"]["cniPlugins"][0]["ready"], false);

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!("{}/api/v1/nodes/node-1", ctx.base_url),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "node-1");

    let response = client
        .get(format!("{}/api/v1/nodes/node-1", ctx.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn storageclass_can_be_created_and_deleted() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/storageclasses",
        &storageclass_manifest("fast-ssd"),
    )
    .await?;

    let fetched = get_json(&client, &ctx.base_url, "/api/v1/storageclasses/fast-ssd").await?;
    assert_eq!(fetched["kind"], "StorageClass");
    assert_eq!(fetched["metadata"]["name"], "fast-ssd");
    assert_eq!(fetched["spec"]["provisioner"], "csi.tugboat.cloud/fast");
    assert_eq!(fetched["spec"]["allowVolumeExpansion"], true);

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!("{}/api/v1/storageclasses/fast-ssd", ctx.base_url),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "fast-ssd");

    let response = client
        .get(format!("{}/api/v1/storageclasses/fast-ssd", ctx.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn clusternetworkclass_supports_status_updates() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/clusternetworkclasses",
        &cluster_network_class_manifest("cluster-shared"),
    )
    .await?;

    let status_patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/clusternetworkclasses/cluster-shared/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "status": {
                "conditions": [
                    {
                        "type": "Ready",
                        "status": "True",
                        "message": "configured"
                    }
                ],
                "readyNodes": ["node-a"]
            }
        })),
    )
    .await?;
    assert_eq!(
        status_patched["status"]["conditions"][0]["message"],
        "configured"
    );
    assert_eq!(status_patched["status"]["readyNodes"][0], "node-a");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/clusternetworkclasses/cluster-shared",
    )
    .await?;
    assert_eq!(fetched["metadata"]["name"], "cluster-shared");
    assert_eq!(fetched["status"]["conditions"][0]["status"], "True");
    assert_eq!(fetched["status"]["readyNodes"][0], "node-a");

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

fn node_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Node",
        "metadata": {
            "name": name,
            "labels": {
                "topology.kubernetes.io/zone": "zone-a"
            }
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

fn storageclass_manifest(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "StorageClass",
        "metadata": {
            "name": name
        },
        "spec": {
            "provisioner": "csi.tugboat.cloud/fast",
            "parameters": {
                "pool": "ssd"
            },
            "reclaimPolicy": "Delete",
            "allowVolumeExpansion": true,
            "mountOptions": ["discard"]
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
