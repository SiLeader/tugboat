use crate::data::{ReadResponse, StatusResponse};
use crate::operator::ApiOperator;
use actix_web::get;
use actix_web::web::{Data, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Namespace;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct ReadParams {
    name: String,
}

#[utoipa::path()]
#[get("/v1/namespaces/{name}")]
pub(super) async fn handle_namespace_read(
    path: Path<ReadParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Namespace>, StatusResponse> {
    let namespace = operator.store.get(None, &path.name).await?;

    match namespace {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(StatusResponse::not_found(
            "Namespace not found",
            Some(serde_json::json!({ "name": path.name })),
        )),
    }
}
