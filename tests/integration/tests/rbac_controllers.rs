#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;
use std::time::Duration;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn service_account_creation_generates_token_secret() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let _controller_manager = ctx.start_controller_manager().await?;
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "controller-ns").await?;

    request_json(
        &client,
        Method::POST,
        &format!(
            "{}/api/v1/namespaces/controller-ns/serviceaccounts",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "ServiceAccount",
            "metadata": {
                "name": "builder",
                "namespace": "controller-ns"
            }
        })),
    )
    .await?;

    let service_account = wait_for_json(
        &client,
        &format!(
            "{}/api/v1/namespaces/controller-ns/serviceaccounts/builder",
            ctx.base_url
        ),
        |body| {
            body["secrets"]
                .as_array()
                .is_some_and(|secrets| !secrets.is_empty())
        },
    )
    .await?;
    let secret_name = service_account["secrets"][0]["name"]
        .as_str()
        .ok_or("service account secret reference is missing name")?;

    let secret = wait_for_json(
        &client,
        &format!(
            "{}/api/v1/namespaces/controller-ns/secrets/{secret_name}",
            ctx.base_url
        ),
        |body| body["data"]["token"].as_str().is_some(),
    )
    .await?;
    assert_eq!(secret["type"], "tugboat.cloud/service-account-token");
    assert_eq!(
        secret["metadata"]["annotations"]["tugboat.cloud/service-account.name"],
        "builder"
    );
    assert!(
        secret["data"]["token"]
            .as_str()
            .is_some_and(|token| !token.is_empty())
    );

    Ok(())
}

#[tokio::test]
async fn namespace_creation_generates_default_service_account() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let _controller_manager = ctx.start_controller_manager().await?;
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "default-sa-ns").await?;

    let service_account = wait_for_json(
        &client,
        &format!(
            "{}/api/v1/namespaces/default-sa-ns/serviceaccounts/default",
            ctx.base_url
        ),
        |body| body["metadata"]["name"] == "default",
    )
    .await?;
    assert_eq!(service_account["metadata"]["namespace"], "default-sa-ns");

    Ok(())
}

#[tokio::test]
async fn aggregated_cluster_role_propagates_child_rules() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let _controller_manager = ctx.start_controller_manager().await?;
    let client = ctx.http_client()?;

    let parent_name = format!(
        "agg-parent-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let child_name = format!("{parent_name}-child");
    let aggregate_label = format!("rbac.tugboat.cloud/{parent_name}");

    request_json(
        &client,
        Method::POST,
        &format!("{}/apis/authorization/v1/clusterroles", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "authorization/v1",
            "kind": "ClusterRole",
            "metadata": { "name": parent_name },
            "aggregationRule": {
                "clusterRoleSelectors": [
                    { "matchLabels": { aggregate_label.clone(): "true" } }
                ]
            }
        })),
    )
    .await?;

    request_json(
        &client,
        Method::POST,
        &format!("{}/apis/authorization/v1/clusterroles", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "authorization/v1",
            "kind": "ClusterRole",
            "metadata": {
                "name": child_name,
                "labels": { aggregate_label.clone(): "true" }
            },
            "rules": [
                {
                    "apiGroups": ["core"],
                    "resources": ["ships"],
                    "verbs": ["get", "list", "watch"]
                }
            ]
        })),
    )
    .await?;

    let parent = wait_for_json(
        &client,
        &format!(
            "{}/apis/authorization/v1/clusterroles/{parent_name}",
            ctx.base_url
        ),
        |body| {
            body["rules"]
                .as_array()
                .map(|rules| {
                    rules.iter().any(|rule| {
                        rule["resources"]
                            .as_array()
                            .is_some_and(|values| values.iter().any(|v| v == "ships"))
                    })
                })
                .unwrap_or(false)
        },
    )
    .await?;
    assert!(parent["aggregationRule"].is_object());

    request_json_with_statuses(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/authorization/v1/clusterroles/{child_name}",
            ctx.base_url
        ),
        &[StatusCode::OK, StatusCode::NO_CONTENT],
        None,
    )
    .await?;

    wait_for_json(
        &client,
        &format!(
            "{}/apis/authorization/v1/clusterroles/{parent_name}",
            ctx.base_url
        ),
        |body| {
            body["rules"]
                .as_array()
                .map(|rules| {
                    !rules.iter().any(|rule| {
                        rule["resources"]
                            .as_array()
                            .is_some_and(|values| values.iter().any(|v| v == "ships"))
                    })
                })
                .unwrap_or(true)
        },
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

async fn create_namespace(
    client: &SecureClient,
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

async fn request_json(
    client: &SecureClient,
    method: Method,
    url: &str,
    expected_status: StatusCode,
    body: Option<Value>,
) -> Result<Value, DynError> {
    request_json_with_statuses(client, method, url, &[expected_status], body).await
}

async fn request_json_with_statuses(
    client: &SecureClient,
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
        "unexpected HTTP status"
    );
    Ok(response.json().await?)
}

async fn wait_for_json(
    client: &SecureClient,
    url: &str,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value, DynError> {
    for _ in 0..50 {
        let response = client.get(url).send().await?;
        if response.status() == StatusCode::OK {
            let body: Value = response.json().await?;
            if predicate(&body) {
                return Ok(body);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    Err("condition was not satisfied before timeout".into())
}
