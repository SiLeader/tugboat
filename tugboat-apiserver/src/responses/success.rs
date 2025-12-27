use crate::responses::StatusResponse;
use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::Serialize;
use serde_json::json;

pub(crate) enum CreateResponse<T> {
    Created(T),
}

impl<T: Serialize> Responder for CreateResponse<T> {
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        match self {
            CreateResponse::Created(body) => match serde_json::to_string(&body) {
                Ok(body) => HttpResponse::Created().body(body),
                Err(_) => StatusResponse::internal_error("Cannot serialize response body", None)
                    .respond_to(req),
            },
        }
    }
}
