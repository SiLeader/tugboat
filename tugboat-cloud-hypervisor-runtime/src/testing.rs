// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::{CloudHypervisorBootConfig, CloudHypervisorVmConfig};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

pub(crate) struct TestVm {
    pub config: CloudHypervisorVmConfig,
    pub id: String,
    base_dir: PathBuf,
}

impl TestVm {
    pub fn new(prefix: &str, id: &str) -> Self {
        let base_dir = unique_dir(prefix);
        std::fs::create_dir_all(&base_dir).unwrap();
        Self {
            config: CloudHypervisorVmConfig {
                executable: "/usr/bin/cloud-hypervisor".into(),
                disk_image_location: base_dir.to_string_lossy().into_owned(),
                boot: CloudHypervisorBootConfig {
                    kernel: None,
                    initramfs: None,
                    firmware: None,
                },
            },
            id: id.into(),
            base_dir,
        }
    }

    pub fn socket_path(&self) -> PathBuf {
        self.base_dir.join(format!("{}.ch.sock", self.id))
    }

    pub fn runtime_request_path(&self, name: &str) -> PathBuf {
        self.base_dir.join(name)
    }
}

impl Drop for TestVm {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.socket_path());
        let _ = std::fs::remove_dir_all(&self.base_dir);
    }
}

#[derive(Debug)]
pub(crate) struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// Spawns a mock Unix-socket HTTP server that replies to each incoming
/// request with the corresponding entry in `responses`, in order. After
/// the last response is delivered the server closes the connection.
pub(crate) fn spawn_mock_server(
    socket_path: PathBuf,
    responses: Vec<String>,
) -> tokio::task::JoinHandle<Vec<CapturedRequest>> {
    let listener = UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut requests = Vec::new();

        for response in responses {
            let Some(request) = read_request(&mut reader).await else {
                break;
            };
            requests.push(request);
            writer.write_all(response.as_bytes()).await.unwrap();
            writer.flush().await.unwrap();
        }

        requests
    })
}

async fn read_request(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> Option<CapturedRequest> {
    let mut request_line = String::new();
    let bytes = reader.read_line(&mut request_line).await.unwrap();
    if bytes == 0 {
        return None;
    }

    let request_line = request_line.trim_end_matches(['\r', '\n']);
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap().to_string();
    let path = request_parts.next().unwrap().to_string();

    let headers = read_headers(reader).await;
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await.unwrap();
    }

    Some(CapturedRequest {
        method,
        path,
        headers,
        body,
    })
}

async fn read_headers(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return headers;
        }
        let (name, value) = trimmed.split_once(':').unwrap();
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
}

pub(crate) fn http_empty_response(status: &str) -> String {
    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n")
}

pub(crate) fn http_json_response(status: &str, body: Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Generic response builder for tests that need to send an arbitrary
/// body without going through `serde_json::Value`.
pub(crate) fn http_response(status: &str, body: Option<&str>) -> String {
    let body = body.unwrap_or("");
    let content_type = if body.is_empty() {
        String::new()
    } else {
        "Content-Type: application/json\r\n".to_string()
    };
    format!(
        "HTTP/1.1 {status}\r\n{content_type}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

pub(crate) fn decode_json(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap()
}

fn unique_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
