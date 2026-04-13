#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

struct RoleRuleInput {
    api_groups: Value,
    resources: Value,
    verbs: Value,
    resource_names: Option<Value>,
}

#[tokio::test]
async fn bearer_token_authentication_succeeds_and_invalid_token_is_rejected() -> Result<(), DynError>
{
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    let allowed = admin
        .get(format!("{}/api/v1/namespaces", ctx.base_url))
        .send()
        .await?;
    assert_eq!(allowed.status(), StatusCode::OK);

    let invalid = ctx.bearer_client("not-a-real-token")?;
    let denied = invalid
        .get(format!("{}/api/v1/namespaces", ctx.base_url))
        .send()
        .await?;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn unreferenced_service_account_token_secret_is_rejected() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    create_namespace(&admin, &ctx.base_url, "forged").await?;
    request_json(
        &admin,
        Method::POST,
        &format!("{}/api/v1/namespaces/forged/serviceaccounts", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "ServiceAccount",
            "metadata": {
                "name": "reader",
                "namespace": "forged"
            }
        })),
    )
    .await?;
    request_json(
        &admin,
        Method::POST,
        &format!("{}/api/v1/namespaces/forged/secrets", ctx.base_url),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": "reader-token",
                "namespace": "forged",
                "annotations": {
                    "tugboat.cloud/service-account.name": "reader"
                }
            },
            "type": "tugboat.cloud/service-account-token",
            "stringData": {
                "token": "forged-reader-token"
            }
        })),
    )
    .await?;

    let forged = ctx.bearer_client("forged-reader-token")?;
    let response = forged
        .get(format!("{}/api/v1/namespaces", ctx.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn anonymous_requests_are_processed_as_anonymous_and_denied_by_rbac() -> Result<(), DynError>
{
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    let response = client
        .get(format!("{}/api/v1/namespaces", ctx.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn role_binding_grants_namespaced_access_only_to_bound_subjects() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    create_namespace(&admin, &ctx.base_url, "team-a").await?;
    create_configmap(&admin, &ctx.base_url, "team-a", "app-config").await?;

    let reader = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "team-a",
        "reader",
        ctx.ca_cert_pem(),
    )
    .await?;
    let outsider = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "team-a",
        "outsider",
        ctx.ca_cert_pem(),
    )
    .await?;

    let denied = reader
        .get(format!(
            "{}/api/v1/namespaces/team-a/configmaps/app-config",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    create_role(
        &admin,
        &ctx.base_url,
        "team-a",
        "config-reader",
        RoleRuleInput {
            api_groups: json!(["core"]),
            resources: json!(["configmaps"]),
            verbs: json!(["get"]),
            resource_names: None,
        },
    )
    .await?;
    create_role_binding(
        &admin,
        &ctx.base_url,
        "team-a",
        "reader-binding",
        "Role",
        "config-reader",
        ("team-a", "reader"),
    )
    .await?;

    let allowed = reader
        .get(format!(
            "{}/api/v1/namespaces/team-a/configmaps/app-config",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(allowed.status(), StatusCode::OK);

    let outsider_denied = outsider
        .get(format!(
            "{}/api/v1/namespaces/team-a/configmaps/app-config",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(outsider_denied.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn cluster_role_binding_grants_access_across_namespaces() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    for namespace in ["ns-a", "ns-b"] {
        create_namespace(&admin, &ctx.base_url, namespace).await?;
        create_configmap(&admin, &ctx.base_url, namespace, "shared").await?;
    }

    let reader = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "ns-a",
        "cluster-reader",
        ctx.ca_cert_pem(),
    )
    .await?;
    create_cluster_role(
        &admin,
        &ctx.base_url,
        "cluster-config-reader",
        json!(["core"]),
        json!(["configmaps"]),
        json!(["get"]),
        None,
    )
    .await?;
    create_cluster_role_binding(
        &admin,
        &ctx.base_url,
        "cluster-config-reader-binding",
        "cluster-config-reader",
        "ns-a",
        "cluster-reader",
    )
    .await?;

    for namespace in ["ns-a", "ns-b"] {
        let response = reader
            .get(format!(
                "{}/api/v1/namespaces/{namespace}/configmaps/shared",
                ctx.base_url
            ))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }

    Ok(())
}

#[tokio::test]
async fn role_binding_to_cluster_role_is_limited_to_its_namespace() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    for namespace in ["blue", "green"] {
        create_namespace(&admin, &ctx.base_url, namespace).await?;
        create_configmap(&admin, &ctx.base_url, namespace, "shared").await?;
    }

    let reader = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "blue",
        "ns-reader",
        ctx.ca_cert_pem(),
    )
    .await?;
    create_cluster_role(
        &admin,
        &ctx.base_url,
        "scoped-cluster-reader",
        json!(["core"]),
        json!(["configmaps"]),
        json!(["get"]),
        None,
    )
    .await?;
    create_role_binding(
        &admin,
        &ctx.base_url,
        "blue",
        "cluster-role-binding",
        "ClusterRole",
        "scoped-cluster-reader",
        ("blue", "ns-reader"),
    )
    .await?;

    let blue = reader
        .get(format!(
            "{}/api/v1/namespaces/blue/configmaps/shared",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(blue.status(), StatusCode::OK);

    let green = reader
        .get(format!(
            "{}/api/v1/namespaces/green/configmaps/shared",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(green.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn wildcard_and_resource_name_rules_are_enforced() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    create_namespace(&admin, &ctx.base_url, "wild").await?;
    create_configmap(&admin, &ctx.base_url, "wild", "allowed").await?;
    create_configmap(&admin, &ctx.base_url, "wild", "blocked").await?;
    create_secret(&admin, &ctx.base_url, "wild", "allowed-secret").await?;

    let wildcard_user = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "wild",
        "wildcard",
        ctx.ca_cert_pem(),
    )
    .await?;
    create_role(
        &admin,
        &ctx.base_url,
        "wild",
        "wildcard-role",
        RoleRuleInput {
            api_groups: json!(["*"]),
            resources: json!(["*"]),
            verbs: json!(["*"]),
            resource_names: None,
        },
    )
    .await?;
    create_role_binding(
        &admin,
        &ctx.base_url,
        "wild",
        "wildcard-binding",
        "Role",
        "wildcard-role",
        ("wild", "wildcard"),
    )
    .await?;

    let secret_read = wildcard_user
        .get(format!(
            "{}/api/v1/namespaces/wild/secrets/allowed-secret",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(secret_read.status(), StatusCode::OK);

    let named_user = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "wild",
        "named-reader",
        ctx.ca_cert_pem(),
    )
    .await?;
    create_role(
        &admin,
        &ctx.base_url,
        "wild",
        "named-role",
        RoleRuleInput {
            api_groups: json!(["core"]),
            resources: json!(["configmaps"]),
            verbs: json!(["get"]),
            resource_names: Some(json!(["allowed"])),
        },
    )
    .await?;
    create_role_binding(
        &admin,
        &ctx.base_url,
        "wild",
        "named-binding",
        "Role",
        "named-role",
        ("wild", "named-reader"),
    )
    .await?;

    let allowed = named_user
        .get(format!(
            "{}/api/v1/namespaces/wild/configmaps/allowed",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(allowed.status(), StatusCode::OK);

    let blocked = named_user
        .get(format!(
            "{}/api/v1/namespaces/wild/configmaps/blocked",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn get_only_role_denies_create_update_and_delete_operations() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    create_namespace(&admin, &ctx.base_url, "write-deny").await?;
    create_configmap(&admin, &ctx.base_url, "write-deny", "existing").await?;

    let reader = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "write-deny",
        "reader-only",
        ctx.ca_cert_pem(),
    )
    .await?;
    create_role(
        &admin,
        &ctx.base_url,
        "write-deny",
        "read-only",
        RoleRuleInput {
            api_groups: json!(["core"]),
            resources: json!(["configmaps"]),
            verbs: json!(["get", "list"]),
            resource_names: None,
        },
    )
    .await?;
    create_role_binding(
        &admin,
        &ctx.base_url,
        "write-deny",
        "read-only-binding",
        "Role",
        "read-only",
        ("write-deny", "reader-only"),
    )
    .await?;

    // GET should succeed
    let get_response = reader
        .get(format!(
            "{}/api/v1/namespaces/write-deny/configmaps/existing",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(get_response.status(), StatusCode::OK);

    // POST (create) should be denied
    let create_response = reader
        .post(format!(
            "{}/api/v1/namespaces/write-deny/configmaps",
            ctx.base_url
        ))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": "new-map", "namespace": "write-deny" },
            "data": { "key": "value" }
        }))
        .send()
        .await?;
    assert_eq!(create_response.status(), StatusCode::FORBIDDEN);

    // PUT (update) should be denied
    let update_response = reader
        .put(format!(
            "{}/api/v1/namespaces/write-deny/configmaps/existing",
            ctx.base_url
        ))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": "existing", "namespace": "write-deny" },
            "data": { "key": "updated" }
        }))
        .send()
        .await?;
    assert_eq!(update_response.status(), StatusCode::FORBIDDEN);

    // DELETE should be denied
    let delete_response = reader
        .delete(format!(
            "{}/api/v1/namespaces/write-deny/configmaps/existing",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(delete_response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn write_role_allows_create_but_still_denies_delete() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let admin = ctx.admin_client()?;
    create_namespace(&admin, &ctx.base_url, "write-allow").await?;

    let writer = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "write-allow",
        "writer",
        ctx.ca_cert_pem(),
    )
    .await?;
    create_role(
        &admin,
        &ctx.base_url,
        "write-allow",
        "create-only",
        RoleRuleInput {
            api_groups: json!(["core"]),
            resources: json!(["configmaps"]),
            verbs: json!(["create", "get"]),
            resource_names: None,
        },
    )
    .await?;
    create_role_binding(
        &admin,
        &ctx.base_url,
        "write-allow",
        "create-only-binding",
        "Role",
        "create-only",
        ("write-allow", "writer"),
    )
    .await?;

    // POST (create) should succeed
    let create_response = writer
        .post(format!(
            "{}/api/v1/namespaces/write-allow/configmaps",
            ctx.base_url
        ))
        .json(&json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": "writer-map", "namespace": "write-allow" },
            "data": { "key": "value" }
        }))
        .send()
        .await?;
    assert!(
        create_response.status() == StatusCode::OK
            || create_response.status() == StatusCode::CREATED,
        "expected 200 or 201, got {}",
        create_response.status()
    );

    // DELETE should be denied
    let delete_response = writer
        .delete(format!(
            "{}/api/v1/namespaces/write-allow/configmaps/writer-map",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(delete_response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn system_masters_client_certificate_bypasses_rbac_checks() -> Result<(), DynError> {
    let Some(ctx) = setup_mtls_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.masters_client()?;
    let response = client
        .get(format!("{}/api/v1/namespaces", ctx.base_url))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

async fn setup_or_skip() -> Result<Option<TestContext>, DynError> {
    let Some(ctx) = TestContext::setup_rbac().await? else {
        eprintln!(
            "skipping integration test: set TUGBOAT_TEST_APISERVER_URL or TUGBOAT_TEST_ETCD_ENDPOINT (or install docker) to enable"
        );
        return Ok(None);
    };
    Ok(Some(ctx))
}

async fn setup_mtls_or_skip() -> Result<Option<TestContext>, DynError> {
    let Some(ctx) = TestContext::setup_rbac_with_mtls().await? else {
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

async fn create_configmap(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
) -> Result<(), DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces/{namespace}/configmaps"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": name,
                "namespace": namespace
            },
            "data": {
                "key": "value"
            }
        })),
    )
    .await?;
    Ok(())
}

async fn create_secret(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
) -> Result<(), DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces/{namespace}/secrets"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": name,
                "namespace": namespace
            },
            "stringData": {
                "token": "secret"
            }
        })),
    )
    .await?;
    Ok(())
}

async fn create_service_account_with_token(
    admin: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
    ca_cert_pem: Option<&[u8]>,
) -> Result<SecureClient, DynError> {
    let mut service_account = request_json(
        admin,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces/{namespace}/serviceaccounts"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "ServiceAccount",
            "metadata": {
                "name": name,
                "namespace": namespace
            }
        })),
    )
    .await?;

    let token = format!("{namespace}-{name}-token");
    let secret = request_json(
        admin,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces/{namespace}/secrets"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": format!("{name}-token"),
                "namespace": namespace,
                "annotations": {
                    "tugboat.cloud/service-account.name": name
                }
            },
            "type": "tugboat.cloud/service-account-token",
            "stringData": {
                "token": token
            }
        })),
    )
    .await?;
    let secret_uid = secret["metadata"]["uid"]
        .as_str()
        .ok_or("service account token secret is missing metadata.uid")?;
    service_account["secrets"] = json!([{
        "kind": "Secret",
        "namespace": namespace,
        "name": format!("{name}-token"),
        "uid": secret_uid,
        "apiVersion": "v1"
    }]);
    request_json(
        admin,
        Method::PUT,
        &format!("{base_url}/api/v1/namespaces/{namespace}/serviceaccounts/{name}"),
        StatusCode::OK,
        Some(service_account),
    )
    .await?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    let mut builder = reqwest::Client::builder().default_headers(headers);
    if let Some(ca_pem) = ca_cert_pem {
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(ca_pem)?);
    }
    Ok(SecureClient::new(builder.build()?))
}

async fn create_role(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
    rule_input: RoleRuleInput,
) -> Result<(), DynError> {
    let mut rule = json!({
        "apiGroups": rule_input.api_groups,
        "resources": rule_input.resources,
        "verbs": rule_input.verbs
    });
    if let Some(resource_names) = rule_input.resource_names {
        rule["resourceNames"] = resource_names;
    }

    request_json(
        client,
        Method::POST,
        &format!("{base_url}/apis/authorization/v1/namespaces/{namespace}/roles"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "authorization/v1",
            "kind": "Role",
            "metadata": {
                "name": name,
                "namespace": namespace
            },
            "rules": [rule]
        })),
    )
    .await?;
    Ok(())
}

async fn create_cluster_role(
    client: &SecureClient,
    base_url: &str,
    name: &str,
    api_groups: Value,
    resources: Value,
    verbs: Value,
    resource_names: Option<Value>,
) -> Result<(), DynError> {
    let mut rule = json!({
        "apiGroups": api_groups,
        "resources": resources,
        "verbs": verbs
    });
    if let Some(resource_names) = resource_names {
        rule["resourceNames"] = resource_names;
    }

    request_json(
        client,
        Method::POST,
        &format!("{base_url}/apis/authorization/v1/clusterroles"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "authorization/v1",
            "kind": "ClusterRole",
            "metadata": {
                "name": name
            },
            "rules": [rule]
        })),
    )
    .await?;
    Ok(())
}

async fn create_role_binding(
    client: &SecureClient,
    base_url: &str,
    namespace: &str,
    name: &str,
    kind: &str,
    role_name: &str,
    subject: (&str, &str),
) -> Result<(), DynError> {
    let (subject_namespace, subject_name) = subject;
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/apis/authorization/v1/namespaces/{namespace}/rolebindings"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "authorization/v1",
            "kind": "RoleBinding",
            "metadata": {
                "name": name,
                "namespace": namespace
            },
            "subjects": [
                {
                    "kind": "ServiceAccount",
                    "name": subject_name,
                    "namespace": subject_namespace,
                    "apiGroup": "authorization"
                }
            ],
            "roleRef": {
                "apiGroup": "authorization",
                "kind": kind,
                "name": role_name
            }
        })),
    )
    .await?;
    Ok(())
}

async fn create_cluster_role_binding(
    client: &SecureClient,
    base_url: &str,
    name: &str,
    role_name: &str,
    subject_namespace: &str,
    subject_name: &str,
) -> Result<(), DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/apis/authorization/v1/clusterrolebindings"),
        StatusCode::OK,
        Some(json!({
            "apiVersion": "authorization/v1",
            "kind": "ClusterRoleBinding",
            "metadata": {
                "name": name
            },
            "subjects": [
                {
                    "kind": "ServiceAccount",
                    "name": subject_name,
                    "namespace": subject_namespace,
                    "apiGroup": "authorization"
                }
            ],
            "roleRef": {
                "apiGroup": "authorization",
                "kind": "ClusterRole",
                "name": role_name
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
        "unexpected status: expected one of {accepted:?}, got {}",
        response.status()
    );
    Ok(response.json().await?)
}
