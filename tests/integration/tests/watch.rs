#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;
use std::time::Duration;

use helpers::setup::SecureClient;
use helpers::setup::TestContext;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

#[tokio::test]
async fn ship_watch_emits_added_events() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let mut watch = start_watch(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("watch", "True")],
    )
    .await?;

    let created = create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "watched-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let event = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(event["type"], "ADDED");
    assert_eq!(event["object"]["metadata"]["name"], "watched-ship");
    assert_eq!(
        event["object"]["metadata"]["resourceVersion"],
        created["metadata"]["resourceVersion"]
    );
    assert_eq!(
        event["object"]["metadata"]["generation"],
        created["metadata"]["generation"]
    );
    assert_eq!(
        event["object"]["spec"]["image"],
        "registry.example.com/demo:v1"
    );

    Ok(())
}

#[tokio::test]
async fn ship_watch_emits_modified_events() -> Result<(), DynError> {
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
            "watched-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let mut watch = start_watch(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("watch", "True")],
    )
    .await?;

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/watched-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "example.com/owner": "watch-test"
                }
            }
        })),
    )
    .await?;

    let event = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(event["type"], "MODIFIED");
    assert_eq!(
        event["object"]["metadata"]["annotations"]["example.com/owner"],
        "watch-test"
    );
    assert_ne!(
        event["object"]["metadata"]["resourceVersion"],
        created["metadata"]["resourceVersion"]
    );
    assert_eq!(
        event["object"]["metadata"]["generation"],
        created["metadata"]["generation"]
    );

    Ok(())
}

#[tokio::test]
async fn ship_watch_emits_deleted_events() -> Result<(), DynError> {
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
            "watched-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let mut watch = start_watch(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("watch", "True")],
    )
    .await?;

    request_json(
        &client,
        Method::DELETE,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/watched-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        None,
    )
    .await?;

    let event = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(event["type"], "DELETED");
    assert_eq!(event["object"]["metadata"]["name"], "watched-ship");

    Ok(())
}

#[tokio::test]
async fn ship_watch_respects_resource_version() -> Result<(), DynError> {
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
            "watched-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;
    let resource_version = created["metadata"]["resourceVersion"]
        .as_str()
        .ok_or("created ship is missing metadata.resourceVersion")?
        .to_string();

    request_json(
        &client,
        Method::PATCH,
        &format!(
            "{}/api/v1/namespaces/test-ns/ships/watched-ship",
            ctx.base_url
        ),
        StatusCode::OK,
        Some(json!({
            "metadata": {
                "annotations": {
                    "example.com/revision": "updated"
                }
            }
        })),
    )
    .await?;

    let mut watch = start_watch(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("watch", "True"), ("resourceVersion", &resource_version)],
    )
    .await?;

    let event = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(event["type"], "MODIFIED");
    assert_eq!(
        event["object"]["metadata"]["annotations"]["example.com/revision"],
        "updated"
    );
    assert_ne!(
        event["object"]["metadata"]["resourceVersion"],
        Value::String(resource_version)
    );

    Ok(())
}

#[tokio::test]
async fn ship_watch_respects_label_selectors() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let mut watch = start_watch(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("watch", "True"), ("labelSelector", "app=watched")],
    )
    .await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest_with_labels(
            "test-ns",
            "ignored-ship",
            "small",
            "registry.example.com/demo:ignored",
            &[],
        ),
    )
    .await?;
    assert!(
        watch
            .read_event(Duration::from_millis(750))
            .await
            .is_err_and(|err| err.to_string().contains("timed out"))
    );

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest_with_labels(
            "test-ns",
            "matched-ship",
            "small",
            "registry.example.com/demo:matched",
            &[("app", "watched")],
        ),
    )
    .await?;

    let event = watch.read_event(Duration::from_secs(5)).await?;
    assert_eq!(event["type"], "ADDED");
    assert_eq!(event["object"]["metadata"]["name"], "matched-ship");

    Ok(())
}

#[tokio::test]
async fn ship_watch_supports_ndjson_mode() -> Result<(), DynError> {
    let Some(ctx) = setup_or_skip().await? else {
        return Ok(());
    };

    let client = ctx.http_client()?;
    create_namespace(&client, &ctx.base_url, "test-ns").await?;

    let mut watch = start_watch(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &[("watch", "NdJson")],
    )
    .await?;

    create_resource(
        &client,
        &ctx.base_url,
        "/api/v1/namespaces/test-ns/ships",
        &ship_manifest(
            "test-ns",
            "ndjson-ship",
            "small",
            "registry.example.com/demo:v1",
        ),
    )
    .await?;

    let line = watch.read_line(Duration::from_secs(5)).await?;
    assert!(line.contains("\"type\":\"ADDED\""));
    let event: Value = serde_json::from_str(&line)?;
    assert_eq!(event["object"]["metadata"]["name"], "ndjson-ship");

    Ok(())
}

struct WatchStream {
    response: Response,
    buffer: Vec<u8>,
}

impl WatchStream {
    async fn read_event(&mut self, timeout: Duration) -> Result<Value, DynError> {
        let line = self.read_line(timeout).await?;
        Ok(serde_json::from_str(&line)?)
    }

    async fn read_line(&mut self, timeout: Duration) -> Result<String, DynError> {
        tokio::time::timeout(timeout, self.read_line_inner())
            .await
            .map_err(|_| -> DynError { "watch read timed out".into() })?
    }

    async fn read_line_inner(&mut self) -> Result<String, DynError> {
        loop {
            if let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line = self.buffer.drain(..=newline).collect::<Vec<_>>();
                let line = String::from_utf8(line)?;
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                return Ok(line.to_string());
            }

            let chunk = self
                .response
                .chunk()
                .await?
                .ok_or("watch stream ended before an event was received")?;
            self.buffer.extend_from_slice(&chunk);
        }
    }
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

async fn start_watch(
    client: &SecureClient,
    base_url: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<WatchStream, DynError> {
    let url = if query.is_empty() {
        format!("{base_url}{path}")
    } else {
        let query = query
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        format!("{base_url}{path}?{query}")
    };
    let response = client.get(&url).send().await?;
    let status = response.status();
    assert_eq!(status, StatusCode::OK);
    Ok(WatchStream {
        response,
        buffer: Vec::new(),
    })
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
    ship_manifest_with_labels(namespace, name, ship_class, image, &[])
}

fn ship_manifest_with_labels(
    namespace: &str,
    name: &str,
    ship_class: &str,
    image: &str,
    labels: &[(&str, &str)],
) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Ship",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": labels_to_json(labels)
        },
        "spec": {
            "image": image,
            "shipClass": ship_class
        }
    })
}

fn labels_to_json(labels: &[(&str, &str)]) -> Value {
    Value::Object(
        labels
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::String((*value).to_string())))
            .collect(),
    )
}
