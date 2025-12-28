use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::Serialize;

pub(crate) enum CreateResponse<T> {
    Created(T),
}

impl<T: Serialize> Responder for CreateResponse<T> {
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        match self {
            CreateResponse::Created(body) => HttpResponse::Created().json(body),
        }
    }
}
