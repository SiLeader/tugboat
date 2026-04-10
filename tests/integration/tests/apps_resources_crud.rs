#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn replicaset_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets",
        &replicaset_manifest("test-ns", "demo-rs", 2, "example.com/images/demo:v1"),
    )
    .await?;
    assert_eq!(created["kind"], "ReplicaSet");
    assert_eq!(created["metadata"]["name"], "demo-rs");
    assert_eq!(created["spec"]["replicas"], 2);
    assert_eq!(created["spec"]["selector"]["app"], "demo");
    assert_eq!(
        created["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/demo:v1"
    );

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets/demo-rs",
    )
    .await?;
    assert_eq!(fetched["metadata"]["namespace"], "test-ns");
    assert_eq!(
        fetched["spec"]["shipTemplate"]["spec"]["shipClass"],
        "standard"
    );

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/replicasets/demo-rs",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "replicas": 4
            }
        })),
    )
    .await?;
    assert_eq!(patched["spec"]["replicas"], 4);

    let refetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/replicasets/demo-rs",
    )
    .await?;
    assert_eq!(refetched["spec"]["replicas"], 4);

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/replicasets/demo-rs",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "demo-rs");

    Ok(())
}

#[tokio::test]
async fn deployment_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/deployments",
        &deployment_manifest(
            "test-ns",
            "demo-deployment",
            3,
            "example.com/images/demo:v1",
        ),
    )
    .await?;
    assert_eq!(created["kind"], "Deployment");
    assert_eq!(created["spec"]["replicas"], 3);
    assert_eq!(created["spec"]["strategy"]["type"], "RollingUpdate");
    assert_eq!(created["spec"]["strategy"]["rollingUpdate"]["maxSurge"], 1);

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/deployments/demo-deployment",
    )
    .await?;
    assert_eq!(fetched["spec"]["selector"]["app"], "demo");
    assert_eq!(
        fetched["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/demo:v1"
    );

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/demo-deployment",
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
    assert_eq!(patched["spec"]["replicas"], 5);

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/deployments/demo-deployment",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(deployment_manifest(
            "test-ns",
            "demo-deployment",
            2,
            "example.com/images/demo:v2",
        )),
    )
    .await?;
    assert_eq!(replaced["spec"]["replicas"], 2);
    assert_eq!(
        replaced["spec"]["shipTemplate"]["spec"]["image"],
        "example.com/images/demo:v2"
    );

    let deleted = request_json(
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
    assert_eq!(deleted["metadata"]["name"], "demo-deployment");

    Ok(())
}

#[tokio::test]
async fn fleet_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets",
        &fleet_manifest(
            "test-ns",
            "demo-fleet",
            "tenant-net",
            &[("frontend", 2, "example.com/images/frontend:v1")],
        ),
    )
    .await?;
    assert_eq!(created["kind"], "Fleet");
    assert_eq!(created["spec"]["networkClassName"], "tenant-net");
    assert_eq!(created["spec"]["components"][0]["name"], "frontend");
    assert_eq!(created["spec"]["components"][0]["replicas"], 2);

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/test-ns/fleets/demo-fleet",
    )
    .await?;
    assert_eq!(
        fetched["spec"]["components"][0]["shipTemplate"]["spec"]["image"],
        "example.com/images/frontend:v1"
    );

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/apps/v1/namespaces/test-ns/fleets/demo-fleet",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "spec": {
                "components": [
                    {
                        "name": "frontend",
                        "replicas": 3,
                        "shipTemplate": {
                            "spec": {
                                "image": "example.com/images/frontend:v2",
                                "shipClass": "standard"
                            }
                        }
                    },
                    {
                        "name": "worker",
                        "replicas": 1,
                        "shipTemplate": {
                            "spec": {
                                "image": "example.com/images/worker:v1",
                                "shipClass": "standard"
                            }
                        }
                    }
                ]
            }
        })),
    )
    .await?;
    assert_eq!(patched["spec"]["components"][0]["replicas"], 3);
    assert_eq!(patched["spec"]["components"][1]["name"], "worker");

    let deleted = request_json(
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
    assert_eq!(deleted["metadata"]["name"], "demo-fleet");

    Ok(())
}

#[tokio::test]
async fn apps_list_all_returns_resources_from_multiple_namespaces() -> Result<(), DynError> {
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
        "/apis/apps/v1/namespaces/ns-a/replicasets",
        &replicaset_manifest("ns-a", "rs-a", 1, "example.com/images/rs-a:v1"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/ns-b/replicasets",
        &replicaset_manifest("ns-b", "rs-b", 1, "example.com/images/rs-b:v1"),
    )
    .await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/ns-a/deployments",
        &deployment_manifest("ns-a", "dep-a", 1, "example.com/images/dep-a:v1"),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/ns-b/deployments",
        &deployment_manifest("ns-b", "dep-b", 1, "example.com/images/dep-b:v1"),
    )
    .await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/ns-a/fleets",
        &fleet_manifest(
            "ns-a",
            "fleet-a",
            "tenant-a",
            &[("frontend", 1, "example.com/images/a:v1")],
        ),
    )
    .await?;
    create_resource(
        &client,
        &ctx.base_url,
        "/apis/apps/v1/namespaces/ns-b/fleets",
        &fleet_manifest(
            "ns-b",
            "fleet-b",
            "tenant-b",
            &[("frontend", 1, "example.com/images/b:v1")],
        ),
    )
    .await?;

    let replicaset_list = get_json(&client, &ctx.base_url, "/apis/apps/v1/replicasets").await?;
    assert_list_contains(&replicaset_list, &[("ns-a", "rs-a"), ("ns-b", "rs-b")])?;

    let deployment_list = get_json(&client, &ctx.base_url, "/apis/apps/v1/deployments").await?;
    assert_list_contains(&deployment_list, &[("ns-a", "dep-a"), ("ns-b", "dep-b")])?;

    let fleet_list = get_json(&client, &ctx.base_url, "/apis/apps/v1/fleets").await?;
    assert_list_contains(&fleet_list, &[("ns-a", "fleet-a"), ("ns-b", "fleet-b")])?;

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

fn assert_list_contains(list: &Value, expected: &[(&str, &str)]) -> Result<(), DynError> {
    assert_eq!(list["kind"], "List");
    let items = list["items"]
        .as_array()
        .ok_or("list response is missing items")?;

    for (namespace, name) in expected {
        assert!(
            items.iter().any(|item| {
                item["metadata"]["namespace"] == *namespace && item["metadata"]["name"] == *name
            }),
            "list did not include {namespace}/{name}"
        );
    }

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

fn deployment_manifest(namespace: &str, name: &str, replicas: i32, image: &str) -> Value {
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
                "app": "demo"
            },
            "shipTemplate": {
                "metadata": {
                    "labels": {
                        "app": "demo"
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
