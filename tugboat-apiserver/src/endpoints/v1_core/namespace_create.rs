use crate::responses::{CreateResponse, StatusResponse};
use actix_web::post;
use tugboat_resources::manifests::core::v1::Namespace;

#[post("/v1/namespaces")]
pub(super) async fn handle_namespace_create() -> Result<CreateResponse<Namespace>, StatusResponse> {
    todo!()
}
