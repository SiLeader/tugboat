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

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmSnapshotMode {
    #[default]
    Online,
    Offline,
}

impl fmt::Display for VmSnapshotMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotCreateRequest {
    pub ship_id: String,
    #[serde(default)]
    pub mode: VmSnapshotMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotCreateResponse {
    pub handle: String,
    pub runtime: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotDeleteRequest {
    pub ship_id: String,
    pub handle: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotDeleteResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotRestoreRequest {
    pub ship_id: String,
    pub handle: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotRestoreResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotListRequest {
    pub ship_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotListResponse {
    pub snapshots: Vec<VmSnapshotEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSnapshotEntry {
    pub handle: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotValidationError {
    field: &'static str,
    message: String,
}

impl SnapshotValidationError {
    fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }

    pub fn field(&self) -> &'static str {
        self.field
    }
}

impl fmt::Display for SnapshotValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for SnapshotValidationError {}

fn validate_ship_id(id: &str) -> Result<(), SnapshotValidationError> {
    if id.is_empty() {
        return Err(SnapshotValidationError::new("shipId", "must not be empty"));
    }
    if id.starts_with('.') {
        return Err(SnapshotValidationError::new(
            "shipId",
            "must not start with '.'",
        ));
    }
    if id.contains("..") {
        return Err(SnapshotValidationError::new(
            "shipId",
            "must not contain '..'",
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(SnapshotValidationError::new(
            "shipId",
            "contains invalid characters; only alphanumeric, '-', '_', and '.' are allowed",
        ));
    }
    Ok(())
}

fn validate_handle(handle: &str) -> Result<(), SnapshotValidationError> {
    if handle.is_empty() {
        return Err(SnapshotValidationError::new("handle", "must not be empty"));
    }
    if handle.starts_with('.') {
        return Err(SnapshotValidationError::new(
            "handle",
            "must not start with '.'",
        ));
    }
    if handle.contains("..") {
        return Err(SnapshotValidationError::new(
            "handle",
            "must not contain '..'",
        ));
    }
    if handle.contains('/') {
        return Err(SnapshotValidationError::new(
            "handle",
            "must not contain '/'",
        ));
    }
    Ok(())
}

impl VmSnapshotCreateRequest {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        validate_ship_id(&self.ship_id)
    }
}

impl VmSnapshotDeleteRequest {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        validate_ship_id(&self.ship_id)?;
        validate_handle(&self.handle)
    }
}

impl VmSnapshotRestoreRequest {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        validate_ship_id(&self.ship_id)?;
        validate_handle(&self.handle)
    }
}

impl VmSnapshotListRequest {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        validate_ship_id(&self.ship_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_mode_serializes_as_lowercase_string() {
        let json = serde_json::to_string(&VmSnapshotMode::Online).unwrap();
        assert_eq!(json, "\"online\"");
        let json = serde_json::to_string(&VmSnapshotMode::Offline).unwrap();
        assert_eq!(json, "\"offline\"");
    }

    #[test]
    fn create_request_serializes_and_validates() {
        let req = VmSnapshotCreateRequest {
            ship_id: "ship-uid".to_string(),
            mode: VmSnapshotMode::Online,
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: VmSnapshotCreateRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.ship_id, "ship-uid");
        assert_eq!(decoded.mode, VmSnapshotMode::Online);
        decoded.validate().unwrap();
    }

    #[test]
    fn create_request_default_mode_is_online() {
        let decoded: VmSnapshotCreateRequest =
            serde_json::from_str(r#"{"shipId":"ship-uid"}"#).unwrap();
        assert_eq!(decoded.mode, VmSnapshotMode::Online);
    }

    #[test]
    fn rejects_empty_ship_id() {
        let req = VmSnapshotCreateRequest {
            ship_id: String::new(),
            mode: VmSnapshotMode::Online,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field(), "shipId");
    }

    #[test]
    fn rejects_path_traversal_ship_id() {
        let req = VmSnapshotCreateRequest {
            ship_id: "../etc".to_string(),
            mode: VmSnapshotMode::Online,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn delete_request_validates_handle() {
        let req = VmSnapshotDeleteRequest {
            ship_id: "ship-uid".to_string(),
            handle: "snap-1".to_string(),
        };
        req.validate().unwrap();

        let req = VmSnapshotDeleteRequest {
            ship_id: "ship-uid".to_string(),
            handle: String::new(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field(), "handle");

        let req = VmSnapshotDeleteRequest {
            ship_id: "ship-uid".to_string(),
            handle: "a/b".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field(), "handle");
    }

    #[test]
    fn restore_request_rejects_dotdot_handle() {
        let req = VmSnapshotRestoreRequest {
            ship_id: "ship-uid".to_string(),
            handle: "..".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn list_response_round_trips() {
        let resp = VmSnapshotListResponse {
            snapshots: vec![VmSnapshotEntry {
                handle: "snap-a".to_string(),
                created_at: "2026-05-14T00:00:00Z".to_string(),
                size_bytes: Some(1024),
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: VmSnapshotListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.snapshots.len(), 1);
        assert_eq!(decoded.snapshots[0].handle, "snap-a");
        assert_eq!(decoded.snapshots[0].size_bytes, Some(1024));
    }
}
