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

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const API_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct ChApiClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl ChApiClient {
    pub async fn connect(socket_path: impl AsRef<Path>) -> crate::Result<Self> {
        let stream = tokio::time::timeout(API_CONNECT_TIMEOUT, UnixStream::connect(socket_path))
            .await
            .map_err(|_| {
                crate::Error::Api("timed out connecting to Cloud Hypervisor API socket".into())
            })??;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
        })
    }

    pub async fn get(&mut self, path: &str) -> crate::Result<Value> {
        Ok(self
            .send_request("GET", path, None)
            .await?
            .unwrap_or(Value::Null))
    }

    pub async fn put(&mut self, path: &str, body: Option<&Value>) -> crate::Result<Option<Value>> {
        self.send_request("PUT", path, body).await
    }

    async fn send_request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> crate::Result<Option<Value>> {
        tokio::time::timeout(
            API_REQUEST_TIMEOUT,
            self.send_request_inner(method, path, body),
        )
        .await
        .map_err(|_| {
            crate::Error::Api(format!(
                "timed out after {API_REQUEST_TIMEOUT:.0?} waiting for Cloud Hypervisor API {method} {path}"
            ))
        })?
    }

    async fn send_request_inner(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> crate::Result<Option<Value>> {
        let body = body.map(serde_json::to_vec).transpose()?;
        let content_length = body.as_ref().map_or(0, Vec::len);

        let mut request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n");
        if body.is_some() {
            request.push_str("Content-Type: application/json\r\n");
        }
        request.push_str(&format!("Content-Length: {content_length}\r\n\r\n"));

        self.writer.write_all(request.as_bytes()).await?;
        if let Some(body) = body {
            self.writer.write_all(&body).await?;
        }
        self.writer.flush().await?;

        let response = self.read_response().await?;
        if !(200..300).contains(&response.status_code) {
            let message = match response.body {
                Some(body) => format!(
                    "Cloud Hypervisor API returned HTTP {} {}: {}",
                    response.status_code,
                    response.reason_phrase,
                    String::from_utf8_lossy(&body)
                ),
                None => format!(
                    "Cloud Hypervisor API returned HTTP {} {}",
                    response.status_code, response.reason_phrase
                ),
            };
            return Err(crate::Error::Api(message));
        }

        match response.body {
            Some(body) => Ok(Some(serde_json::from_slice(&body)?)),
            None => Ok(None),
        }
    }

    async fn read_response(&mut self) -> crate::Result<HttpResponse> {
        let status_line = self.read_line().await?;
        let mut status_parts = status_line.splitn(3, ' ');
        let protocol = status_parts.next().unwrap_or_default();
        let status_code = status_parts
            .next()
            .ok_or_else(|| {
                crate::Error::Api(format!("invalid HTTP response status line: {status_line}"))
            })?
            .parse::<u16>()
            .map_err(|_| {
                crate::Error::Api(format!("invalid HTTP response status line: {status_line}"))
            })?;
        let reason_phrase = status_parts.next().unwrap_or_default().to_string();

        if !protocol.starts_with("HTTP/1.") {
            return Err(crate::Error::Api(format!(
                "unsupported HTTP response protocol: {protocol}"
            )));
        }

        let headers = self.read_headers().await?;
        let content_length = headers
            .get("content-length")
            .map(|value| {
                value.parse::<usize>().map_err(|_| {
                    crate::Error::Api(format!("invalid Content-Length header value: {value}"))
                })
            })
            .transpose()?
            .unwrap_or(0);

        let body = if content_length == 0 {
            None
        } else {
            let mut body = vec![0; content_length];
            self.reader.read_exact(&mut body).await?;
            Some(body)
        };

        Ok(HttpResponse {
            status_code,
            reason_phrase,
            body,
        })
    }

    async fn read_line(&mut self) -> crate::Result<String> {
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line).await?;
            if bytes == 0 {
                return Err(crate::Error::Api(
                    "unexpected EOF while reading HTTP response".into(),
                ));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            return Ok(trimmed.to_string());
        }
    }

    async fn read_headers(&mut self) -> crate::Result<HashMap<String, String>> {
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line).await?;
            if bytes == 0 {
                return Err(crate::Error::Api(
                    "unexpected EOF while reading HTTP headers".into(),
                ));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                return Ok(headers);
            }
            let (name, value) = trimmed
                .split_once(':')
                .ok_or_else(|| crate::Error::Api(format!("invalid HTTP header line: {trimmed}")))?;
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
}

struct HttpResponse {
    status_code: u16,
    reason_phrase: String,
    body: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::ChApiClient;
    use crate::Error;
    use crate::testing::{TestVm, http_response, spawn_mock_server};
    use serde_json::json;

    #[tokio::test]
    async fn test_get_request() {
        let response_body = json!({
            "state": "Running",
            "vcpus": 2,
        });
        let test_vm = TestVm::new("tugboat-ch-api", "vm-get");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(&response_body.to_string()))],
        );

        let mut client = ChApiClient::connect(test_vm.socket_path()).await.unwrap();
        let response = client.get("/api/v1/vm.info").await.unwrap();
        let requests = server.await.unwrap();

        assert_eq!(response, response_body);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/v1/vm.info");
        assert_eq!(
            request.headers.get("host").map(String::as_str),
            Some("localhost")
        );
        assert_eq!(
            request.headers.get("content-length").map(String::as_str),
            Some("0")
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn test_put_request_with_body() {
        let request_body = json!({
            "desired_vcpus": 4,
            "desired_ram": 8589934592_u64,
        });
        let response_body = json!({
            "result": "ok",
        });
        let test_vm = TestVm::new("tugboat-ch-api", "vm-put");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("200 OK", Some(&response_body.to_string()))],
        );

        let mut client = ChApiClient::connect(test_vm.socket_path()).await.unwrap();
        let response = client
            .put("/api/v1/vm.resize", Some(&request_body))
            .await
            .unwrap();
        let requests = server.await.unwrap();

        assert_eq!(response, Some(response_body));
        let request = &requests[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.path, "/api/v1/vm.resize");
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
            request_body
        );
    }

    #[tokio::test]
    async fn test_put_request_without_body() {
        let test_vm = TestVm::new("tugboat-ch-api", "vm-put-empty");
        let server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response("204 No Content", None)],
        );

        let mut client = ChApiClient::connect(test_vm.socket_path()).await.unwrap();
        let response = client.put("/api/v1/vm.shutdown", None).await.unwrap();
        let requests = server.await.unwrap();

        assert_eq!(response, None);
        let request = &requests[0];
        assert_eq!(request.method, "PUT");
        assert_eq!(request.path, "/api/v1/vm.shutdown");
        assert_eq!(
            request.headers.get("content-length").map(String::as_str),
            Some("0")
        );
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn test_error_response() {
        let test_vm = TestVm::new("tugboat-ch-api", "vm-err");
        let _server = spawn_mock_server(
            test_vm.socket_path(),
            vec![http_response(
                "404 Not Found",
                Some("{\"error\":\"missing\"}"),
            )],
        );

        let mut client = ChApiClient::connect(test_vm.socket_path()).await.unwrap();
        let error = client.get("/api/v1/vm.info").await.unwrap_err();

        match error {
            Error::Api(message) => {
                assert!(message.contains("404 Not Found"));
                assert!(message.contains("missing"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
