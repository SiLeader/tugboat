use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::Serialize;

pub(crate) struct ReadResponse<T> {
    data: T,
}

impl<T: Serialize> ReadResponse<T> {
    pub(crate) fn new(data: T) -> Self {
        Self { data }
    }
}

impl<T: Serialize> Responder for ReadResponse<T> {
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        HttpResponse::Ok().json(&self.data)
    }
}
