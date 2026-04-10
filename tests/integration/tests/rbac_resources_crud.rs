#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn role_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "rbac-ns").await?;

    let created = request_json(
        &client,
        Method::POST,
        &format!(
            "{}/apis/authorization/v1/namespaces/rbac-ns/roles",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(role_manifest("rbac-ns", "configmap-reader", "get")),
    )
    .await?;
    assert_eq!(created["metadata"]["name"], "configmap-reader");
    assert_eq!(created["rules"][0]["resources"][0], "configmaps");

    let listed = get_json(
        &client,
        &ctx.base_url,
        "/apis/authorization/v1/namespaces/rbac-ns/roles",
    )
    .await?;
    assert_contains_named_item(&listed, "configmap-reader")?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/authorization/v1/namespaces/rbac-ns/roles/configmap-reader",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "labels": {
                    "stage": "patched"
                }
            }
        })),
    )
    .await?;
    assert_eq!(patched["metadata"]["labels"]["stage"], "patched");

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/apis/authorization/v1/namespaces/rbac-ns/roles/configmap-reader",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(role_manifest("rbac-ns", "configmap-reader", "list")),
    )
    .await?;
    assert_eq!(replaced["rules"][0]["verbs"][0], "list");
    assert_eq!(replaced["metadata"]["labels"]["stage"], "patched");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/authorization/v1/namespaces/rbac-ns/roles/configmap-reader",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "configmap-reader");

    Ok(())
}

#[tokio::test]
async fn cluster_role_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;

    let created = request_json(
        &client,
        Method::POST,
        &format!("{}/apis/authorization/v1/clusterroles", ctx.base_url),
        StatusCode::OK,
        Some(cluster_role_manifest("fleet-reader", "fleets", "get")),
    )
    .await?;
    assert_eq!(created["metadata"]["name"], "fleet-reader");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/authorization/v1/clusterroles/fleet-reader",
    )
    .await?;
    assert_eq!(fetched["rules"][0]["resources"][0], "fleets");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/authorization/v1/clusterroles/fleet-reader",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "updated-by": "integration-test"
                }
            }
        })),
    )
    .await?;
    assert_eq!(
        patched["metadata"]["annotations"]["updated-by"],
        "integration-test"
    );

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/apis/authorization/v1/clusterroles/fleet-reader",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(cluster_role_manifest("fleet-reader", "fleets", "list")),
    )
    .await?;
    assert_eq!(replaced["rules"][0]["verbs"][0], "list");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/authorization/v1/clusterroles/fleet-reader",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "fleet-reader");

    Ok(())
}

#[tokio::test]
async fn role_binding_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "bindings-ns").await?;
    request_json(
        &client,
        Method::POST,
        &format!(
            "{}/apis/authorization/v1/namespaces/bindings-ns/roles",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(role_manifest("bindings-ns", "reader", "get")),
    )
    .await?;

    let created = request_json(
        &client,
        Method::POST,
        &format!(
            "{}/apis/authorization/v1/namespaces/bindings-ns/rolebindings",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(role_binding_manifest(
            "bindings-ns",
            "reader-binding",
            "Role",
            "reader",
        )),
    )
    .await?;
    assert_eq!(created["metadata"]["name"], "reader-binding");
    assert_eq!(created["subjects"][0]["kind"], "ServiceAccount");

    let listed = get_json(
        &client,
        &ctx.base_url,
        "/apis/authorization/v1/namespaces/bindings-ns/rolebindings",
    )
    .await?;
    assert_contains_named_item(&listed, "reader-binding")?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/authorization/v1/namespaces/bindings-ns/rolebindings/reader-binding",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "labels": {
                    "owner": "qa"
                }
            }
        })),
    )
    .await?;
    assert_eq!(patched["metadata"]["labels"]["owner"], "qa");

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/apis/authorization/v1/namespaces/bindings-ns/rolebindings/reader-binding",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(role_binding_manifest(
            "bindings-ns",
            "reader-binding",
            "Role",
            "reader",
        )),
    )
    .await?;
    assert_eq!(replaced["roleRef"]["name"], "reader");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/authorization/v1/namespaces/bindings-ns/rolebindings/reader-binding",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "reader-binding");

    Ok(())
}

#[tokio::test]
async fn cluster_role_binding_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    request_json(
        &client,
        Method::POST,
        &format!("{}/apis/authorization/v1/clusterroles", ctx.base_url),
        StatusCode::OK,
        Some(cluster_role_manifest(
            "cluster-reader",
            "namespaces",
            "list",
        )),
    )
    .await?;

    let created = request_json(
        &client,
        Method::POST,
        &format!("{}/apis/authorization/v1/clusterrolebindings", ctx.base_url),
        StatusCode::OK,
        Some(cluster_role_binding_manifest(
            "cluster-reader-binding",
            "cluster-reader",
        )),
    )
    .await?;
    assert_eq!(created["metadata"]["name"], "cluster-reader-binding");

    let fetched = get_json(
        &client,
        &ctx.base_url,
        "/apis/authorization/v1/clusterrolebindings/cluster-reader-binding",
    )
    .await?;
    assert_eq!(fetched["roleRef"]["kind"], "ClusterRole");

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/apis/authorization/v1/clusterrolebindings/cluster-reader-binding",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "scope": "cluster"
                }
            }
        })),
    )
    .await?;
    assert_eq!(patched["metadata"]["annotations"]["scope"], "cluster");

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/apis/authorization/v1/clusterrolebindings/cluster-reader-binding",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(cluster_role_binding_manifest(
            "cluster-reader-binding",
            "cluster-reader",
        )),
    )
    .await?;
    assert_eq!(replaced["subjects"][0]["name"], "workload");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/authorization/v1/clusterrolebindings/cluster-reader-binding",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "cluster-reader-binding");

    Ok(())
}

#[tokio::test]
async fn service_account_supports_crud_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "serviceaccounts-ns").await?;

    let created = request_json(
        &client,
        Method::POST,
        &format!(
            "{}/api/v1/namespaces/serviceaccounts-ns/serviceaccounts",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(service_account_manifest("serviceaccounts-ns", "builder")),
    )
    .await?;
    assert_eq!(created["metadata"]["name"], "builder");

    let listed = get_json(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/serviceaccounts-ns/serviceaccounts",
    )
    .await?;
    assert_contains_named_item(&listed, "builder")?;

    let patched = request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/serviceaccounts-ns/serviceaccounts/builder",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "team": "platform"
                }
            }
        })),
    )
    .await?;
    assert_eq!(patched["metadata"]["annotations"]["team"], "platform");

    let replaced = request_json(
        &client,
        Method::PUT,
        &format!(
            "{}/api/v1/namespaces/serviceaccounts-ns/serviceaccounts/builder",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(service_account_manifest("serviceaccounts-ns", "builder")),
    )
    .await?;
    assert_eq!(replaced["metadata"]["name"], "builder");

    let deleted = request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/serviceaccounts-ns/serviceaccounts/builder",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;
    assert_eq!(deleted["metadata"]["name"], "builder");

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
    client: &Client,
    base_url: &str,
    namespace: &str,
) -> Result<(), DynError> {
    request_json_with_statuses(
        client,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces"),
        &[StatusCode::OK, StatusCode::CREATED],
        Some(json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": namespace
            }
        })),
    )
    .await?;
    Ok(())
}

async fn get_json(client: &Client, base_url: &str, path: &str) -> Result<Value, DynError> {
    let response = client.get(format!("{base_url}{path}")).send().await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unexpected status for {path}"
    );
    Ok(response.json().await?)
}

async fn request_json(
    client: &Client,
    method: Method,
    url: &str,
    expected_status: StatusCode,
    body: Option<Value>,
) -> Result<Value, DynError> {
    request_json_with_statuses(client, method, url, &[expected_status], body).await
}

async fn request_json_with_statuses(
    client: &Client,
    method: Method,
    url: &str,
    expected_statuses: &[StatusCode],
    body: Option<Value>,
) -> Result<Value, DynError> {
    let mut accepted = expected_statuses.to_vec();
    if method == Method::POST && accepted == [StatusCode::OK] {
        accepted.push(StatusCode::CREATED);
    }
    let request = client.request(method, url);
    let request = if let Some(body) = body {
        request.json(&body)
    } else {
        request
    };
    let response = request.send().await?;
    assert!(
        accepted.contains(&response.status()),
        "unexpected status for {url}: expected one of {accepted:?}, got {}",
        response.status()
    );
    Ok(response.json().await?)
}

fn assert_contains_named_item(list: &Value, name: &str) -> Result<(), DynError> {
    let items = list["items"]
        .as_array()
        .ok_or("list response is missing items")?;
    assert!(
        items.iter().any(|item| item["metadata"]["name"] == name),
        "list response did not include {name}"
    );
    Ok(())
}

fn role_manifest(namespace: &str, name: &str, verb: &str) -> Value {
    json!({
        "apiVersion": "authorization/v1",
        "kind": "Role",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "rules": [
            {
                "apiGroups": ["core"],
                "resources": ["configmaps"],
                "verbs": [verb]
            }
        ]
    })
}

fn cluster_role_manifest(name: &str, resource: &str, verb: &str) -> Value {
    json!({
        "apiVersion": "authorization/v1",
        "kind": "ClusterRole",
        "metadata": {
            "name": name
        },
        "rules": [
            {
                "apiGroups": ["core"],
                "resources": [resource],
                "verbs": [verb]
            }
        ]
    })
}

fn role_binding_manifest(namespace: &str, name: &str, kind: &str, role_name: &str) -> Value {
    json!({
        "apiVersion": "authorization/v1",
        "kind": "RoleBinding",
        "metadata": {
            "name": name,
            "namespace": namespace
        },
        "subjects": [
            {
                "kind": "ServiceAccount",
                "name": "workload",
                "namespace": namespace,
                "apiGroup": "authorization"
            }
        ],
        "roleRef": {
            "apiGroup": "authorization",
            "kind": kind,
            "name": role_name
        }
    })
}

fn cluster_role_binding_manifest(name: &str, role_name: &str) -> Value {
    json!({
        "apiVersion": "authorization/v1",
        "kind": "ClusterRoleBinding",
        "metadata": {
            "name": name
        },
        "subjects": [
            {
                "kind": "ServiceAccount",
                "name": "workload",
                "namespace": "bindings-ns",
                "apiGroup": "authorization"
            }
        ],
        "roleRef": {
            "apiGroup": "authorization",
            "kind": "ClusterRole",
            "name": role_name
        }
    })
}

fn service_account_manifest(namespace: &str, name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ServiceAccount",
        "metadata": {
            "name": name,
            "namespace": namespace
        }
    })
}
