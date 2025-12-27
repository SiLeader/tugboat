use actix_web::body::BoxBody;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, HttpResponseBuilder, Responder, ResponseError};
use serde::Serialize;
use std::fmt::{Display, Formatter};
pub(crate) use success::CreateResponse;
mod success;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusResponse {
    kind: String,
    api_version: String,
    metadata: serde_json::Value,
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
            kind: "Status".to_string(),
            api_version: "v1".to_string(),
            metadata: serde_json::json!({}),
            status: "Failure".to_string(),
            message,
            reason,
            code,
            details,
        }
    }

    // Client error
    error_entry!(bad_request, "BadRequest", 400);
    error_entry!(unauthorized, "Unauthorized", 401);
    error_entry!(forbidden, "Forbidden", 403);
    error_entry!(not_found, "NotFound", 404);
    error_entry!(conflict, "Conflict", 409);
    error_entry!(invalid, "Invalid", 422);

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
