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

use actix_web::body::BoxBody;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, HttpResponseBuilder, Responder, ResponseError};
use serde::Serialize;
use std::fmt::{Display, Formatter};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, TypeMeta};

#[derive(Debug, Serialize, thiserror::Error, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusResponse {
    #[serde(flatten)]
    type_meta: TypeMeta,
    #[serde(rename = "metadata")]
    object_meta: ObjectMeta,
    status: String,
    message: String,
    reason: String,
    code: u16,
    details: Option<serde_json::Value>,
}

macro_rules! error_entry {
    ($error:ident, $reason:literal, $code:literal) => {
        pub(crate) fn $error(message: impl ToString, details: Option<serde_json::Value>) -> Self {
            Self::with_all(message.to_string(), $reason.to_string(), $code, details)
        }
    };
}

impl StatusResponse {
    pub(crate) fn with_all(
        message: String,
        reason: String,
        code: u16,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self {
            type_meta: TypeMeta {
                api_version: Some("v1".to_string()),
                kind: Some("Status".to_string()),
            },
            object_meta: ObjectMeta::default(),
            status: "Failure".to_string(),
            message,
            reason,
            code,
            details,
        }
    }

    // Client error
    error_entry!(bad_request, "BadRequest", 400);
    // error_entry!(unauthorized, "Unauthorized", 401);
    // error_entry!(forbidden, "Forbidden", 403);
    error_entry!(not_found, "NotFound", 404);
    error_entry!(conflict, "Conflict", 409);
    // error_entry!(invalid, "Invalid", 422);

    // Server error
    error_entry!(internal_error, "InternalError", 500);
}

impl Responder for StatusResponse {
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        self.error_response()
    }
}

impl ResponseError for StatusResponse {
    fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        let Ok(status) = StatusCode::from_u16(self.code) else {
            return HttpResponse::InternalServerError()
                .json(StatusResponse::internal_error("Invalid status code", None));
        };
        HttpResponseBuilder::new(status).json(self)
    }
}

impl Display for StatusResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<tugboat_resource_store::error::Error> for StatusResponse {
    fn from(value: tugboat_resource_store::error::Error) -> Self {
        match value {
            tugboat_resource_store::error::Error::UnsupportedType => {
                StatusResponse::bad_request("Invalid resource type", None)
            }
            tugboat_resource_store::error::Error::ProtobufDeserialization(_) => {
                StatusResponse::internal_error("Failed to deserialize protobuf", None)
            }
            tugboat_resource_store::error::Error::FieldMissing(_) => {
                StatusResponse::bad_request("Missing required field", None)
            }
            tugboat_resource_store::error::Error::Etcd(_) => {
                StatusResponse::internal_error("Etcd access error", None)
            }
            tugboat_resource_store::error::Error::EventEmit(_) => {
                StatusResponse::internal_error("Event emit error", None)
            }
            tugboat_resource_store::error::Error::OptimisticLockFailed(revision) => {
                StatusResponse::conflict(
                    "Resources are conflicted",
                    Some(serde_json::json!({"revision": revision})),
                )
            }
        }
    }
}

impl From<serde_json::Error> for StatusResponse {
    fn from(value: serde_json::Error) -> Self {
        StatusResponse::internal_error(
            "Failed to serialize to JSON",
            Some(serde_json::json!({"error": value.to_string()})),
        )
    }
}
