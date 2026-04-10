#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn ship_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "demo-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    assert_eq!(created["kind"], "Ship");
    assert_eq!(created["metadata"]["name"], "demo-ship");
    assert_eq!(created["spec"]["image"], "registry.example.com/demo:v1");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships/demo-ship",
    )
    .await?;
    assert_eq!(fetched["metadata"]["namespace"], "test-ns");
    assert_eq!(fetched["spec"]["shipClass"], "small");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!("{}/api/v1/namespaces/test-ns/ships/demo-ship", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "example.com/owner": "integration-test"
                }
            }
        })),
    )
    .await?;
    assert_eq!(
        patched["metadata"]["annotations"]["example.com/owner"],
        "integration-test"
    );

    let refetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships/demo-ship",
    )
    .await?;
    assert_eq!(
        refetched["metadata"]["annotations"]["example.com/owner"],
        "integration-test"
    );

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!("{}/api/v1/namespaces/test-ns/ships/demo-ship", ctx.base_url),
        StatusCode::OK,
        Some(ship_manifest(
            "test-ns",
            "demo-ship",
            "medium",
            "registry.example.com/demo:v2",
        )),
    )
    .await?;
    assert_eq!(replaced["spec"]["shipClass"], "medium");
    assert_eq!(replaced["spec"]["image"], "registry.example.com/demo:v2");
    assert_eq!(
        replaced["metadata"]["annotations"]["example.com/owner"],
        "integration-test"
    );

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!("{}/api/v1/namespaces/test-ns/ships/demo-ship", ctx.base_url),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "demo-ship");

    let response = client
        .get(format!(
            "{}/api/v1/namespaces/test-ns/ships/demo-ship",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn ship_supports_status_patch_and_replace() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "status-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/status-ship/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "status": {
                "conditions": [
                    {
                        "status": "Ready",
                        "message": "running",
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
    .await?;
    assert_eq!(patched["status"]["conditions"][0]["message"], "running");
    assert_eq!(patched["status"]["ips"][0]["ipv4"], "10.0.0.10");

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/status-ship/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "Ship",
            "metadata": {
                "name": "status-ship",
                "namespace": "test-ns"
            },
            "status": {
                "conditions": [
                    {
                        "status": "NotReady",
                        "message": "draining",
                        "timestamp": "2023-11-14T22:15:00Z"
                    }
                ],
                "ips": [
                    {
                        "nic": "eth0",
                        "ipv4": "10.0.0.11"
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(replaced["status"]["conditions"][0]["status"], "NotReady");
    assert_eq!(replaced["status"]["ips"][0]["ipv4"], "10.0.0.11");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships/status-ship",
    )
    .await?;
    assert_eq!(fetched["status"]["conditions"][0]["message"], "draining");

    Ok(())
}

#[tokio::test]
async fn configmap_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/configmaps",
        &configmap_manifest(
            "test-ns",
            "app-config",
            &[("mode", "dev"), ("region", "apne1")],
        ),
    )
    .await?;
    assert_eq!(created["data"]["mode"], "dev");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/configmaps/app-config",
    )
    .await?;
    assert_eq!(fetched["data"]["region"], "apne1");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/configmaps/app-config",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "data": {
                "mode": "prod",
                "region": "use1"
            }
        })),
    )
    .await?;
    assert_eq!(patched["data"]["mode"], "prod");
    assert_eq!(patched["data"]["region"], "use1");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/configmaps/app-config",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "app-config");

    Ok(())
}

#[tokio::test]
async fn secret_supports_create_read_and_delete() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/secrets",
        &json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": "registry-credentials",
                "namespace": "test-ns"
            },
            "stringData": {
                "token": "s3cr3t"
            }
        }),
    )
    .await?;

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/secrets/registry-credentials",
    )
    .await?;
    assert_eq!(fetched["data"]["token"], "czNjcjN0");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/secrets/registry-credentials",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "registry-credentials");

    Ok(())
}

#[tokio::test]
async fn persistent_volume_claim_supports_crud_and_status_updates() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims",
        &pvc_manifest("test-ns", "data-disk"),
    )
    .await?;
    assert_eq!(created["spec"]["storageClassName"], "fast-ssd");
    assert_eq!(created["spec"]["requestedCapacityBytes"], 1_073_741_824_i64);

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/persistentvolumeclaims/data-disk",
    )
    .await?;
    assert_eq!(fetched["spec"]["accessModes"][0], "ReadWriteOnce");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/data-disk/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "status": {
                "phase": "Bound",
                "capacityBytes": 1_073_741_824_i64,
                "resizePending": false,
                "conditions": [
                    {
                        "status": "True",
                        "message": "bound",
                        "timestamp": "2023-11-14T22:36:40Z"
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(patched["status"]["phase"], "Bound");
    assert_eq!(patched["status"]["conditions"][0]["message"], "bound");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/persistentvolumeclaims/data-disk",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "data-disk");

    Ok(())
}

#[tokio::test]
async fn networkclass_supports_crud_and_status_updates() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/networkclasses",
        &networkclass_manifest("test-ns", "tenant-net", "10.42.0.0/24", "bridge"),
    )
    .await?;
    assert_eq!(created["spec"]["subnet"], "10.42.0.0/24");
    assert_eq!(created["spec"]["cniPlugin"], "bridge");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/networkclasses/tenant-net",
    )
    .await?;
    assert_eq!(fetched["metadata"]["namespace"], "test-ns");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/networkclasses/tenant-net/status",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "status": {
                "conditions": [
                    {
                        "type": "Ready",
                        "status": "True",
                        "message": "configured",
                        "timestamp": "2023-11-14T22:40:00Z"
                    }
                ],
                "readyNodes": ["node-a"]
            }
        })),
    )
    .await?;
    assert_eq!(patched["status"]["conditions"][0]["type"], "Ready");
    assert_eq!(patched["status"]["readyNodes"][0], "node-a");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/networkclasses/tenant-net",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "tenant-net");

    Ok(())
}

#[tokio::test]
async fn ship_list_all_returns_resources_from_multiple_namespaces() -> Result<(), DynError> {
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
        &ship_manifest("ns-a", "ship-a", "small", "registry.example.com/demo:a"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/ns-b/ships",
        &ship_manifest("ns-b", "ship-b", "small", "registry.example.com/demo:b"),
    )
    .await?;

    let listed = get_json(&client, &ctx.base_url, "/api/v1/ships").await?;
    assert_eq!(listed["kind"], "List");
    let items = listed["items"]
        .as_array()
        .ok_or("ship list response is missing items")?;

    for (namespace, name) in [("ns-a", "ship-a"), ("ns-b", "ship-b")] {
        assert!(
            items.iter().any(|item| {
                item["metadata"]["namespace"] == namespace && item["metadata"]["name"] == name
            }),
            "ship list did not include {namespace}/{name}"
        );
    }

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

fn ship_manifest(namespace: &str, name: &str, ship_class: &str, image: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Ship",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "image": image,
            "shipClass": ship_class
        }
    })
}

fn configmap_manifest(namespace: &str, name: &str, data: &[(&str, &str)]) -> Value {
    let data = data
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::String((*value).to_string())))
        .collect::<serde_json::Map<String, Value>>();
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "data": data
    })
}

fn pvc_manifest(namespace: &str, name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "storageClassName": "fast-ssd",
            "volumeMode": "Filesystem",
            "requestedCapacityBytes": 1_073_741_824_i64
        }
    })
}

fn networkclass_manifest(namespace: &str, name: &str, subnet: &str, cni_plugin: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "NetworkClass",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "spec": {
            "subnet": subnet,
            "routes": [],
            "internetAccess": true,
            "clusterNetwork": false,
            "cniPlugin": cni_plugin
        }
    })
}
