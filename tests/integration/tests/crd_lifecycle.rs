#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;
use std::time::Duration;

use helpers::crd::{
    cluster_crd, create_crd, create_namespace, custom_resource, get_json, merge_patch_json,
    namespaced_crd, request_json, start_watch, wait_for_custom_resource,
    wait_for_custom_resource_removed,
};
use helpers::setup::{SecureClient, TestContext};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn crd_lifecycle_creation_is_reflected_in_discovery() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;

    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd("discovery.example.com", "widgets", "widget", "Widget", true),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "discovery.example.com",
        "v1",
        "widgets",
    )
    .await?;

    let groups = get_json(&client, &ctx.base_url, "/apis").await?;
    assert!(groups["groups"].as_array().is_some_and(|groups| {
        groups
            .iter()
            .any(|group| group["name"] == "discovery.example.com")
    }));

    let resources = get_json(&client, &ctx.base_url, "/apis/discovery.example.com/v1").await?;
    let resources = resources["resources"]
        .as_array()
        .ok_or("discovery response is missing resources")?;
    let widget = resources
        .iter()
        .find(|resource| resource["name"] == "widgets")
        .ok_or("CRD resource is missing")?;
    assert_eq!(widget["namespaced"], true);
    assert_has_verbs(
        widget,
        &[
            "create", "delete", "get", "list", "patch", "update", "watch",
        ],
    )?;
    assert!(
        resources
            .iter()
            .any(|resource| resource["name"] == "widgets/status")
    );

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_custom_resource_can_be_created_within_one_second_of_crd_creation()
-> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "fast-crd").await?;
    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd("fast.example.com", "gadgets", "gadget", "Gadget", false),
    )
    .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let response = client
            .post(format!(
                "{}/apis/fast.example.com/v1/namespaces/fast-crd/gadgets",
                ctx.base_url
            ))
            .json(&custom_resource(
                "fast.example.com/v1",
                "Gadget",
                Some("fast-crd"),
                "demo",
            ))
            .send()
            .await?;
        if response.status() == StatusCode::CREATED {
            return Ok(());
        }
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        if tokio::time::Instant::now() >= deadline {
            panic!("custom resource was not creatable within 1 second");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn crd_lifecycle_cluster_scoped_custom_resource_supports_crud() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_crd(
        &client,
        &ctx.base_url,
        cluster_crd(
            "cluster-crud.example.com",
            "clusterwidgets",
            "clusterwidget",
            "ClusterWidget",
            false,
        ),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "cluster-crud.example.com",
        "v1",
        "clusterwidgets",
    )
    .await?;

    let collection = format!(
        "{}/apis/cluster-crud.example.com/v1/clusterwidgets",
        ctx.base_url
    );
    let item = format!("{collection}/demo");
    let created = request_json(
        &client,
        Method::POST,
        &collection,
        &[StatusCode::CREATED],
        Some(custom_resource(
            "cluster-crud.example.com/v1",
            "ClusterWidget",
            None,
            "demo",
        )),
    )
    .await?;
    assert_eq!(created["metadata"]["name"], "demo");

    let patched = merge_patch_json(
        &client,
        &item,
        &[StatusCode::OK],
        json!({"spec": {"image": "registry.example.com/demo:v2"}}),
    )
    .await?;
    assert_eq!(patched["spec"]["image"], "registry.example.com/demo:v2");

    let listed = request_json(&client, Method::GET, &collection, &[StatusCode::OK], None).await?;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(1));

    let deleted = request_json(&client, Method::DELETE, &item, &[StatusCode::OK], None).await?;
    assert_eq!(deleted["metadata"]["name"], "demo");
    assert_status(&client, Method::GET, &item, StatusCode::NOT_FOUND).await?;

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_namespaced_custom_resource_supports_crud() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "crd-crud").await?;
    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd(
            "namespaced-crud.example.com",
            "widgets",
            "widget",
            "Widget",
            false,
        ),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "namespaced-crud.example.com",
        "v1",
        "widgets",
    )
    .await?;

    let collection = format!(
        "{}/apis/namespaced-crud.example.com/v1/namespaces/crd-crud/widgets",
        ctx.base_url
    );
    let item = format!("{collection}/demo");
    request_json(
        &client,
        Method::POST,
        &collection,
        &[StatusCode::CREATED],
        Some(custom_resource(
            "namespaced-crud.example.com/v1",
            "Widget",
            Some("crd-crud"),
            "demo",
        )),
    )
    .await?;

    let fetched = request_json(&client, Method::GET, &item, &[StatusCode::OK], None).await?;
    assert_eq!(fetched["metadata"]["namespace"], "crd-crud");

    let replacement = json!({
        "apiVersion": "namespaced-crud.example.com/v1",
        "kind": "Widget",
        "metadata": {"name": "demo", "namespace": "crd-crud"},
        "spec": {"size": 2, "image": "registry.example.com/demo:v3"}
    });
    let replaced = request_json(
        &client,
        Method::PUT,
        &item,
        &[StatusCode::OK],
        Some(replacement),
    )
    .await?;
    assert_eq!(replaced["spec"]["size"], 2);

    request_json(&client, Method::DELETE, &item, &[StatusCode::OK], None).await?;
    assert_status(&client, Method::GET, &item, StatusCode::NOT_FOUND).await?;

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_custom_resource_validation_and_scope_errors_are_reported()
-> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "crd-validation").await?;
    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd(
            "validation.example.com",
            "widgets",
            "widget",
            "Widget",
            false,
        ),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "validation.example.com",
        "v1",
        "widgets",
    )
    .await?;

    let mut invalid = custom_resource(
        "validation.example.com/v1",
        "Widget",
        Some("crd-validation"),
        "bad",
    );
    invalid["spec"]["size"] = json!(0);
    let invalid_response = request_json(
        &client,
        Method::POST,
        &format!(
            "{}/apis/validation.example.com/v1/namespaces/crd-validation/widgets",
            ctx.base_url
        ),
        &[StatusCode::UNPROCESSABLE_ENTITY],
        Some(invalid),
    )
    .await?;
    assert_eq!(invalid_response["reason"], "Invalid");

    let scope_response = request_json(
        &client,
        Method::POST,
        &format!("{}/apis/validation.example.com/v1/widgets", ctx.base_url),
        &[StatusCode::BAD_REQUEST],
        Some(custom_resource(
            "validation.example.com/v1",
            "Widget",
            None,
            "wrong-scope",
        )),
    )
    .await?;
    assert_eq!(scope_response["reason"], "BadRequest");

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_status_subresource_only_updates_status_when_enabled() -> Result<(), DynError>
{
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "crd-status").await?;
    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd("status.example.com", "widgets", "widget", "Widget", true),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "status.example.com",
        "v1",
        "widgets",
    )
    .await?;

    let collection = format!(
        "{}/apis/status.example.com/v1/namespaces/crd-status/widgets",
        ctx.base_url
    );
    let item = format!("{collection}/demo");
    request_json(
        &client,
        Method::POST,
        &collection,
        &[StatusCode::CREATED],
        Some(custom_resource(
            "status.example.com/v1",
            "Widget",
            Some("crd-status"),
            "demo",
        )),
    )
    .await?;

    // PATCH /status must only contain the status field. Non-status keys must
    // be rejected so clients cannot accidentally mutate spec via the status
    // subresource.
    merge_patch_json(
        &client,
        &format!("{item}/status"),
        &[StatusCode::BAD_REQUEST],
        json!({
            "status": {"phase": "Ready"},
            "spec": {"size": 99}
        }),
    )
    .await?;

    let patched = merge_patch_json(
        &client,
        &format!("{item}/status"),
        &[StatusCode::OK],
        json!({"status": {"phase": "Ready"}}),
    )
    .await?;
    assert_eq!(patched["status"]["phase"], "Ready");
    assert_eq!(patched["spec"]["size"], 1);

    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd("nostatus.example.com", "gadgets", "gadget", "Gadget", false),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "nostatus.example.com",
        "v1",
        "gadgets",
    )
    .await?;
    let no_status_collection = format!(
        "{}/apis/nostatus.example.com/v1/namespaces/crd-status/gadgets",
        ctx.base_url
    );
    request_json(
        &client,
        Method::POST,
        &no_status_collection,
        &[StatusCode::CREATED],
        Some(custom_resource(
            "nostatus.example.com/v1",
            "Gadget",
            Some("crd-status"),
            "demo",
        )),
    )
    .await?;
    assert_status(
        &client,
        Method::PATCH,
        &format!("{no_status_collection}/demo/status"),
        StatusCode::NOT_FOUND,
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_custom_resource_watch_emits_added_modified_deleted_events()
-> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "crd-watch").await?;
    create_crd(
        &client,
        &ctx.base_url,
        namespaced_crd("watch.example.com", "widgets", "widget", "Widget", false),
    )
    .await?;
    wait_for_custom_resource(&client, &ctx.base_url, "watch.example.com", "v1", "widgets").await?;

    let path = "/apis/watch.example.com/v1/namespaces/crd-watch/widgets";
    let collection = format!("{}{}", ctx.base_url, path);
    let mut watch = start_watch(&client, &ctx.base_url, path).await?;
    request_json(
        &client,
        Method::POST,
        &collection,
        &[StatusCode::CREATED],
        Some(custom_resource(
            "watch.example.com/v1",
            "Widget",
            Some("crd-watch"),
            "watched",
        )),
    )
    .await?;
    let added = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(added["type"], "ADDED");

    merge_patch_json(
        &client,
        &format!("{collection}/watched"),
        &[StatusCode::OK],
        json!({"metadata": {"annotations": {"example.com/revision": "2"}}}),
    )
    .await?;
    let modified = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(modified["type"], "MODIFIED");

    request_json(
        &client,
        Method::DELETE,
        &format!("{collection}/watched"),
        &[StatusCode::OK],
        None,
    )
    .await?;
    let deleted = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(deleted["type"], "DELETED");

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_deleting_crd_cascades_custom_resource_data() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;
    create_crd(
        &client,
        &ctx.base_url,
        cluster_crd(
            "delete-crd.example.com",
            "widgets",
            "widget",
            "Widget",
            false,
        ),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "delete-crd.example.com",
        "v1",
        "widgets",
    )
    .await?;

    request_json(
        &client,
        Method::POST,
        &format!("{}/apis/delete-crd.example.com/v1/widgets", ctx.base_url),
        &[StatusCode::CREATED],
        Some(custom_resource(
            "delete-crd.example.com/v1",
            "Widget",
            None,
            "cascade-target",
        )),
    )
    .await?;
    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/apis/apiextensions/v1/customresourcedefinitions/widgets.delete-crd.example.com",
            ctx.base_url
        ),
        &[StatusCode::OK],
        None,
    )
    .await?;

    // The CRD definition is removed from the registry first, so the CR API
    // returns 404 promptly.
    wait_for_custom_resource_removed(
        &client,
        &ctx.base_url,
        "delete-crd.example.com",
        "v1",
        "widgets",
    )
    .await?;

    // Recreate the CRD with the same group/plural; if cascade-delete worked
    // the prior CR data is gone, so the LIST is empty. If we had left the
    // data orphaned (the previous behavior), it would resurface here.
    create_crd(
        &client,
        &ctx.base_url,
        cluster_crd(
            "delete-crd.example.com",
            "widgets",
            "widget",
            "Widget",
            false,
        ),
    )
    .await?;
    wait_for_custom_resource(
        &client,
        &ctx.base_url,
        "delete-crd.example.com",
        "v1",
        "widgets",
    )
    .await?;
    let list = request_json(
        &client,
        Method::GET,
        &format!("{}/apis/delete-crd.example.com/v1/widgets", ctx.base_url),
        &[StatusCode::OK],
        None,
    )
    .await?;
    let items = list["items"].as_array().map(Vec::len).unwrap_or(usize::MAX);
    assert_eq!(items, 0, "cascade delete must remove CR data");

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_rbac_denies_unpermitted_custom_resource_access() -> Result<(), DynError> {
    let Some(ctx) = setup_rbac_or_skip().await? else {
        return Ok(());
    };
    let admin = ctx.admin_client()?;
    create_namespace(&admin, &ctx.base_url, "crd-rbac").await?;
    create_crd(
        &admin,
        &ctx.base_url,
        namespaced_crd("rbac-crd.example.com", "widgets", "widget", "Widget", false),
    )
    .await?;
    wait_for_custom_resource(
        &admin,
        &ctx.base_url,
        "rbac-crd.example.com",
        "v1",
        "widgets",
    )
    .await?;

    request_json(
        &admin,
        Method::POST,
        &format!(
            "{}/apis/rbac-crd.example.com/v1/namespaces/crd-rbac/widgets",
            ctx.base_url
        ),
        &[StatusCode::CREATED],
        Some(custom_resource(
            "rbac-crd.example.com/v1",
            "Widget",
            Some("crd-rbac"),
            "private",
        )),
    )
    .await?;

    let reader = create_service_account_with_token(
        &admin,
        &ctx.base_url,
        "crd-rbac",
        "reader",
        ctx.ca_cert_pem(),
    )
    .await?;
    let denied = reader
        .get(format!(
            "{}/apis/rbac-crd.example.com/v1/namespaces/crd-rbac/widgets/private",
            ctx.base_url
        ))
        .send()
        .await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn crd_lifecycle_reserved_group_crd_is_rejected() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };
    let client = ctx.http_client()?;

    let response = request_json(
        &client,
        Method::POST,
        &format!(
            "{}/apis/apiextensions/v1/customresourcedefinitions",
            ctx.base_url
        ),
        &[StatusCode::UNPROCESSABLE_ENTITY],
        Some(namespaced_crd("core", "widgets", "widget", "Widget", false)),
    )
    .await?;
    assert_eq!(response["reason"], "Invalid");

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

async fn setup_rbac_or_skip() -> Result<Option<TestContext>, DynError> {
    let Some(ctx) = TestContext::setup_rbac().await? else {
        eprintln!(
            "skipping integration test: set TUGBOAT_TEST_ETCD_ENDPOINT (or install docker) to enable"
        );
        return Ok(None);
    };
    Ok(Some(ctx))
}

async fn assert_status(
    client: &SecureClient,
    method: Method,
    url: &str,
    expected: StatusCode,
) -> Result<(), DynError> {
    let response = if method == Method::PATCH {
        client
            .patch(url)
            .header("content-type", "application/merge-patch+json")
            .json(&json!({"status": {"phase": "Ready"}}))
            .send()
            .await?
    } else {
        client.request(method, url).send().await?
    };
    assert_eq!(response.status(), expected);
    Ok(())
}

fn assert_has_verbs(resource: &Value, expected: &[&str]) -> Result<(), DynError> {
    let verbs = resource["verbs"]
        .as_array()
        .ok_or("discovery resource is missing verbs")?;
    for verb in expected {
        assert!(
            verbs.iter().any(|candidate| candidate == verb),
            "resource {} is missing verb {}",
            resource["name"],
            verb
        );
    }
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
        &[StatusCode::OK, StatusCode::CREATED],
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
        &[StatusCode::OK, StatusCode::CREATED],
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
        &[StatusCode::OK],
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
