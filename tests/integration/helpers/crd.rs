#![allow(dead_code)]

use std::error::Error;
use std::time::Duration;

use crate::helpers::setup::SecureClient;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

type DynError = Box<dyn Error + Send + Sync>;

pub struct WatchStream {
    response: Response,
    buffer: Vec<u8>,
}

impl WatchStream {
    pub async fn read_event(&mut self, timeout: Duration) -> Result<Value, DynError> {
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
                if !line.is_empty() {
                    return Ok(line.to_string());
                }
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

pub async fn create_namespace(
    client: &SecureClient,
    base_url: &str,
    name: &str,
) -> Result<Value, DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/api/v1/namespaces"),
        &[StatusCode::OK, StatusCode::CREATED],
        Some(json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": name
            }
        })),
    )
    .await
}

pub async fn create_crd(
    client: &SecureClient,
    base_url: &str,
    manifest: Value,
) -> Result<Value, DynError> {
    request_json(
        client,
        Method::POST,
        &format!("{base_url}/apis/apiextensions/v1/customresourcedefinitions"),
        &[StatusCode::OK, StatusCode::CREATED],
        Some(manifest),
    )
    .await
}

pub async fn wait_for_custom_resource(
    client: &SecureClient,
    base_url: &str,
    group: &str,
    version: &str,
    plural: &str,
) -> Result<(), DynError> {
    let path = format!("/apis/{group}/{version}");
    for _ in 0..20 {
        let response = client.get(format!("{base_url}{path}")).send().await?;
        if response.status() == StatusCode::OK {
            let body: Value = response.json().await?;
            if body["resources"].as_array().is_some_and(|resources| {
                resources.iter().any(|resource| resource["name"] == plural)
            }) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Err(format!("custom resource {group}/{version}/{plural} did not appear in discovery").into())
}

pub async fn wait_for_custom_resource_removed(
    client: &SecureClient,
    base_url: &str,
    group: &str,
    version: &str,
    plural: &str,
) -> Result<(), DynError> {
    let path = format!("/apis/{group}/{version}/{plural}");
    for _ in 0..20 {
        let response = client.get(format!("{base_url}{path}")).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Err(format!("custom resource {group}/{version}/{plural} was still routable").into())
}

pub async fn request_json(
    client: &SecureClient,
    method: Method,
    url: &str,
    expected_statuses: &[StatusCode],
    body: Option<Value>,
) -> Result<Value, DynError> {
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request.send().await?;
    assert_status(response, expected_statuses).await
}

pub async fn merge_patch_json(
    client: &SecureClient,
    url: &str,
    expected_statuses: &[StatusCode],
    body: Value,
) -> Result<Value, DynError> {
    let response = client
        .patch(url)
        .header(CONTENT_TYPE, "application/merge-patch+json")
        .json(&body)
        .send()
        .await?;
    assert_status(response, expected_statuses).await
}

pub async fn get_json(
    client: &SecureClient,
    base_url: &str,
    path: &str,
) -> Result<Value, DynError> {
    request_json(
        client,
        Method::GET,
        &format!("{base_url}{path}"),
        &[StatusCode::OK],
        None,
    )
    .await
}

pub async fn start_watch(
    client: &SecureClient,
    base_url: &str,
    path: &str,
) -> Result<WatchStream, DynError> {
    let response = client
        .get(format!("{base_url}{path}?watch=True"))
        .send()
        .await?;
    let status = response.status();
    assert_eq!(status, StatusCode::OK);
    Ok(WatchStream {
        response,
        buffer: Vec::new(),
    })
}

async fn assert_status(
    response: Response,
    expected_statuses: &[StatusCode],
) -> Result<Value, DynError> {
    let url = response.url().clone();
    let status = response.status();
    let text = response.text().await?;
    assert!(
        expected_statuses.contains(&status),
        "unexpected status for {url}: expected {expected_statuses:?}, got {status}: {text}"
    );
    Ok(serde_json::from_str(&text)?)
}

pub fn namespaced_crd(
    group: &str,
    plural: &str,
    singular: &str,
    kind: &str,
    status_subresource: bool,
) -> Value {
    crd(
        group,
        plural,
        singular,
        kind,
        "Namespaced",
        status_subresource,
    )
}

pub fn cluster_crd(
    group: &str,
    plural: &str,
    singular: &str,
    kind: &str,
    status_subresource: bool,
) -> Value {
    crd(group, plural, singular, kind, "Cluster", status_subresource)
}

fn crd(
    group: &str,
    plural: &str,
    singular: &str,
    kind: &str,
    scope: &str,
    status_subresource: bool,
) -> Value {
    let mut version = json!({
        "name": "v1",
        "served": true,
        "storage": true,
        "schema": {
            "openApiV3Schema": serde_json::to_string(&schema()).expect("schema serializes")
        }
    });
    if status_subresource {
        version["subresources"] = json!({
            "status": {}
        });
    }

    json!({
        "apiVersion": "apiextensions/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {
            "name": format!("{plural}.{group}")
        },
        "spec": {
            "group": group,
            "names": {
                "plural": plural,
                "singular": singular,
                "kind": kind,
                "listKind": format!("{kind}List")
            },
            "scope": scope,
            "versions": [version]
        }
    })
}

pub fn custom_resource(
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> Value {
    let mut metadata = json!({
        "name": name,
        "labels": {
            "app": name
        }
    });
    if let Some(namespace) = namespace {
        metadata["namespace"] = json!(namespace);
    }

    json!({
        "apiVersion": api_version,
        "kind": kind,
        "metadata": metadata,
        "spec": {
            "size": 1,
            "image": "registry.example.com/demo:v1"
        }
    })
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["spec"],
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": {"type": "object"},
            "spec": {
                "type": "object",
                "required": ["size", "image"],
                "properties": {
                    "size": {"type": "integer", "minimum": 1},
                    "image": {"type": "string"}
                }
            },
            "status": {"type": "object"}
        },
        "additionalProperties": false
    })
}
